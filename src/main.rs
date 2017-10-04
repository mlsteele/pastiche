extern crate rand;
extern crate find_folder;
extern crate image;
extern crate time;
extern crate ocl;
#[macro_use] extern crate colorify;

use std::path::Path;
use ocl::{Context, Queue, Device, Program, Image, Kernel};
use ocl::enums::{ImageChannelOrder, ImageChannelDataType, MemObjectType};
use find_folder::Search;
use rand::Rng;
use std::collections::vec_deque::VecDeque;

const MASK_WHITE: image::Luma<u8> = image::Luma{data: [255u8]};
const MASK_BLACK: image::Luma<u8> = image::Luma{data: [0u8]};

fn print_elapsed(title: &str, start: time::Timespec) {
    let time_elapsed = time::get_time() - start;
    let elapsed_ms = time_elapsed.num_milliseconds();
    let separator = if title.len() > 0 { ": " } else { "" };
    println!("    {}{}{}.{:03}", title, separator, time_elapsed.num_seconds(), elapsed_ms);
}

#[allow(dead_code)]
fn read_source_image(loco : &str) -> image::ImageBuffer<image::Rgba<u8>, Vec<u8>> {
    let dyn = image::open(&Path::new(loco)).unwrap();
    let img = dyn.to_rgba();
    img
}

#[allow(dead_code)]
fn min_pixel<I>(img: &I) -> (u32, u32, u16)
    where I: image::GenericImage<Pixel=image::Luma<u16>>
{
    let (x,y,px) = img.pixels().min_by_key(|&(_,_,v)| v.data[0]).unwrap();
    (x, y, px.data[0])
}

// Minimum value pixel in the img.
// Only consider those pixels where map is hot.
// Returns None if there are no viable pixels.
fn min_pixel_with_mask<I,M>(img: &I, mask: &M) -> Option<(u32, u32, u16)>
    where I: image::GenericImage<Pixel=image::Luma<u16>>,
          M: image::GenericImage<Pixel=image::Luma<u8>>
{
    img.pixels().zip(mask.pixels()).filter_map(|((x,y,px), (_,_,map_px))| {
        if map_px.data[0] > 127u8 {
            Some((x,y,px.data[0]))
        } else {
            None
        }
    })
        .min_by_key(|&(_,_,px)| px)
}

// The neighbors of the position that are not filled
// and in bounds.
fn neighbors_empty<M>(x: u32, y: u32, mask_filled: &M) -> Vec<(u32,u32)>
    where M: image::GenericImage<Pixel=image::Luma<u8>>
{
    let x = x as i32;
    let y = y as i32;
    let mut neighbors = vec![];
    let (w, h) = mask_filled.dimensions();
    let w = w as i32;
    let h = h as i32;
    for dx in -1..2 {
        for dy in -1..2 {
            let nx = x + dx;
            let ny = y + dy;
            let is_self = dx == 0 && dy == 0;
            let in_bounds = nx >= 0 && ny >= 0 && nx < w && ny < h;
            // printlnc!(green: "n {} {}    (self:{})  (bounds:{})", nx, ny, is_self, in_bounds);
            if !is_self && in_bounds {
                let nx = nx as u32;
                let ny = ny as u32;
                if mask_filled.get_pixel(nx,ny).data[0] <= 127u8 {
                    neighbors.push((nx, ny));
                }
            }
        }
    }
    neighbors
}

fn main() {
    let compute_program = Search::ParentsThenKids(3, 3)
        .for_folder("cl_src").expect("Error locating 'cl_src'")
        .join("cl/clove.cl");

    println!("getting ocl context...");
    let context = Context::builder().devices(Device::specifier()
        .type_flags(ocl::flags::DEVICE_TYPE_GPU).first()).build().unwrap();
    println!("devices: ({}) {:?}", context.devices().len(), context.devices());
    let device = context.devices()[0];
    println!("device: {:?}", device);
    println!("  {:?}", device.info(ocl::enums::DeviceInfo::Name));
    println!("  {:?}", device.info(ocl::enums::DeviceInfo::Vendor));
    println!("  {:?}", device.info(ocl::enums::DeviceInfo::VendorId));
    println!("  {:?}", device.info(ocl::enums::DeviceInfo::Type));
    println!("  {:?}", device.info(ocl::enums::DeviceInfo::Extensions));
    println!("  {:?}", device.info(ocl::enums::DeviceInfo::OpenclCVersion));
    let queue = Queue::new(&context, device, None).unwrap();

    let program = Program::builder()
        .src_file(compute_program)
        .devices(device)
        .build(&context)
        .unwrap();

    let dims: (u32, u32) = (100, 100);
    let center = (dims.0 / 2, dims.1 / 2);

    let black: image::Rgba<u8> = image::Rgba{data: [0u8, 0u8, 0u8, 255u8]};
    #[allow(unused_variables)]
    let white: image::Rgba<u8> = image::Rgba{data: [255u8, 255u8, 255u8, 255u8]};
    let red: image::Rgba<u8> = image::Rgba{data: [255u8, 0u8, 0u8, 255u8]};

    printlnc!(white_bold: "initializing color queue");
    #[allow(unused_variables)]
    let mut color_queue: VecDeque<image::Rgba<u8>> = {
        let ncolors = (dims.0 * dims.1) as usize;
        let mut q = VecDeque::with_capacity(ncolors);
        for _ in 0..ncolors {
            let mut buf = [0u8; 4];
            rand::thread_rng().fill_bytes(&mut buf);
            buf[3] = 255u8;
            q.push_back(image::Rgba{data: buf});
        }
        q
    };

    let mut img_canvas: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> = image::ImageBuffer::from_pixel(
        dims.0, dims.1, black);

    // Glider:
    // img_canvas.put_pixel(4, 3, white);
    // img_canvas.put_pixel(4, 4, white);
    // img_canvas.put_pixel(4, 5, white);
    // img_canvas.put_pixel(3, 5, white);
    // img_canvas.put_pixel(2, 4, white);

    // Which pixels are filled.
    let mut img_mask_filled: image::ImageBuffer<image::Luma<u8>, Vec<u8>> = image::ImageBuffer::from_pixel(
        dims.0, dims.1, MASK_BLACK);

    // Which pixels are on the frontier.
    let mut img_mask_frontier: image::ImageBuffer<image::Luma<u8>, Vec<u8>> = image::ImageBuffer::from_pixel(
        dims.0, dims.1, MASK_BLACK);

    let mut img_score: image::ImageBuffer<image::Luma<u16>, Vec<u16>> = image::ImageBuffer::from_pixel(
        dims.0, dims.1, image::Luma{data: [0u16]});

    // let mut place_pixel = |x: u32, y: u32, color: image::Rgba<u8>| {
    //     printlnc!(red: "placing {} {}", x, y);
    //     img_canvas.put_pixel(x, y, color);
    //     // Mark as filled
    //     img_mask_filled.put_pixel(x, y, MASK_WHITE);
    //     // Remove from frontier
    //     img_mask_frontier.put_pixel(x, y, MASK_BLACK);
    //     // Add neighbors to frontier
    //     for &(nx, ny) in neighbors_empty(x, y, &img_mask_filled).iter() {
    //         img_mask_frontier.put_pixel(nx, ny, MASK_WHITE);
    //     }
    // };

    fn place_pixel<I,M>(x: u32, y: u32, color: image::Rgba<u8>,
                   canvas: &mut I, mask_filled: &mut M, mask_frontier: &mut M)
        where I: image::GenericImage<Pixel=image::Rgba<u8>>,
              M: image::GenericImage<Pixel=image::Luma<u8>>
    {
        printlnc!(red: "placing {} {}", x, y);
        canvas.put_pixel(x, y, color);
        // Mark as filled
        mask_filled.put_pixel(x, y, MASK_WHITE);
        // Remove from frontier
        // printlnc!(green: "frontier OFF {} {}", x, y);
        mask_frontier.put_pixel(x, y, MASK_BLACK);
        // Add neighbors to frontier
        for &(nx, ny) in neighbors_empty::<M>(x, y, &mask_filled).iter() {
            // printlnc!(green: "frontier ON {} {}", nx, ny);
            mask_frontier.put_pixel(nx, ny, MASK_WHITE);
        }
    };

    place_pixel(center.0, center.1, red,
                &mut img_canvas, &mut img_mask_filled, &mut img_mask_frontier);

    // Initialize the canvas
    // printlnc!(white_bold: "setting up board");
    // for x in 0..dims.0 {
    //     for y in 0..dims.1 {
    //         let mut buf = [0u8; 4];
    //         rand::thread_rng().fill_bytes(&mut buf);
    //         buf[1] = buf[0];
    //         buf[2] = buf[0];
    //         buf[3] = 255u8;
    //         let drop = image::Rgba{data: buf};
    //         img_canvas.put_pixel(x, y, drop);
    //     }
    // }

    printlnc!(white_bold: "saving start image");
    img_canvas.save(&Path::new(&format!("result_{:06}.png", 0))).unwrap();

    for frame in 1..(dims.0 * dims.1) {
        printlnc!(white_bold: "\nFrame: {}", frame);

        let start_time = time::get_time();

        let cl_in_canvas = Image::<u8>::builder()
            .channel_order(ImageChannelOrder::Rgba)
            .channel_data_type(ImageChannelDataType::UnormInt8)
            .image_type(MemObjectType::Image2d)
            .dims(&dims)
            .flags(ocl::flags::MEM_READ_ONLY | ocl::flags::MEM_HOST_WRITE_ONLY | ocl::flags::MEM_COPY_HOST_PTR)
            .queue(queue.clone())
            .host_data(&img_canvas)
            .build().unwrap();

        let cl_in_mask_filled = Image::<u8>::builder()
            .channel_order(ImageChannelOrder::Luminance)
            .channel_data_type(ImageChannelDataType::UnormInt8)
            .image_type(MemObjectType::Image2d)
            .dims(&dims)
            .flags(ocl::flags::MEM_READ_ONLY | ocl::flags::MEM_HOST_WRITE_ONLY | ocl::flags::MEM_COPY_HOST_PTR)
            .queue(queue.clone())
            .host_data(&img_mask_filled)
            .build().unwrap();

        let cl_out_score = Image::<u16>::builder()
            .channel_order(ImageChannelOrder::Luminance)
            .channel_data_type(ImageChannelDataType::UnormInt16)
            .image_type(MemObjectType::Image2d)
            .dims(&dims)
            .flags(ocl::flags::MEM_WRITE_ONLY | ocl::flags::MEM_HOST_READ_ONLY | ocl::flags::MEM_COPY_HOST_PTR)
            .queue(queue.clone())
            .host_data(&img_score)
            .build().unwrap();

        print_elapsed("created memory bindings", start_time);

        let target = color_queue.pop_front();
        if target.is_none() {
            printlnc!(royal_blue: "color queue drained");
            break;
        }
        let target = target.unwrap();
        let goal = ocl::prm::Float4::new(
            (target.data[0] as f32) / 256.,
            (target.data[1] as f32) / 256.,
            (target.data[2] as f32) / 256.,
            (target.data[3] as f32) / 256.
        );

        let kernel = Kernel::new("score", &program).unwrap()
            .queue(queue.clone())
            .gws(&dims)
            .arg_img(&cl_in_canvas)
            .arg_img(&cl_in_mask_filled)
            .arg_vec(goal)
            .arg_img(&cl_out_score);

        printlnc!(royal_blue: "Running kernel...");
        printlnc!(white_bold: "image dims: {:?}", &dims);

        kernel.enq().unwrap();
        print_elapsed("kernel enqueued", start_time);

        queue.finish().unwrap();
        print_elapsed("queue finished", start_time);

        cl_out_score.read(&mut img_score).enq().unwrap();
        print_elapsed("read finished", start_time);

        // img_mask_frontier.save(&Path::new(&format!("mask_frontier_{:06}.png", frame))).unwrap();
        // img_mask_filled.save(&Path::new(&format!("mask_filled_{:06}.png", frame))).unwrap();

        {
            if let Some((x, y, _)) = min_pixel_with_mask(&img_score, &img_mask_frontier) {
                place_pixel(x, y, target,
                            &mut img_canvas, &mut img_mask_filled, &mut img_mask_frontier);
            } else {
                panic!("no viable pixels");
            }
        }

        print_elapsed("placed", start_time);

        if frame % 1000 == 0 {
            img_canvas.save(&Path::new(&format!("result_{:06}.png", frame))).unwrap();

            // {
            //     let buf: Vec<u8> = img_score.clone().into_raw().iter().map(|px| {
            //         (px >> 8) as u8
            //     }).collect();
            //     let img2: image::ImageBuffer<image::Luma<u8>, Vec<u8>> = image::ImageBuffer::from_raw(
            //         dims.0, dims.1, buf).unwrap();
            //     img2
            // }.save(&Path::new(&format!("score_{:06}.png", frame))).unwrap();

            print_elapsed("save", start_time);
        }
    }

    img_canvas.save(&Path::new("result.png")).unwrap();
}

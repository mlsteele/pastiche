extern crate find_folder;
extern crate image;
extern crate time;
extern crate ocl;
#[macro_use] extern crate colorify;

use std::path::Path;
use ocl::{Context, Queue, Device, Program, Image, Kernel};
use ocl::enums::{ImageChannelOrder, ImageChannelDataType, MemObjectType};
use find_folder::Search;
// use image::GenericImage;

fn print_elapsed(title: &str, start: time::Timespec) {
    let time_elapsed = time::get_time() - start;
    let elapsed_ms = time_elapsed.num_milliseconds();
    let separator = if title.len() > 0 { ": " } else { "" };
    println!("    {}{}{}.{:03}", title, separator, time_elapsed.num_seconds(), elapsed_ms);
}


fn read_source_image(loco : &str) -> image::ImageBuffer<image::Rgba<u8>, Vec<u8>> {
    let dyn = image::open(&Path::new(loco)).unwrap();
    let img = dyn.to_rgba();
    img
}


fn main() {
    let compute_program = Search::ParentsThenKids(3, 3)
        .for_folder("cl_src").expect("Error locating 'cl_src'")
        .join("cl/clove.cl");

    let context = Context::builder().devices(Device::specifier()
        .type_flags(ocl::flags::DEVICE_TYPE_GPU).first()).build().unwrap();
    let device = context.devices()[0];
    let queue = Queue::new(&context, device, None).unwrap();

    let program = Program::builder()
        .src_file(compute_program)
        .devices(device)
        .build(&context)
        .unwrap();

    let dims = (1000, 1000);

    let start_pixel: image::Rgba<u8> = image::Rgba{data: [255u8, 255u8, 0u8, 255u8]};
    let mut src_image: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> = image::ImageBuffer::from_pixel(
        dims.0, dims.1, start_pixel);

    // let img = read_source_image("test.jpg");

    // let dims = img.dimensions();

    // ##################################################
    // #################### UNROLLED ####################
    // ##################################################

    let mut result_image: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> = image::ImageBuffer::new(
        dims.0, dims.1);

    let cl_dest = Image::<u8>::builder()
        .channel_order(ImageChannelOrder::Rgba)
        .channel_data_type(ImageChannelDataType::UnormInt8)
        .image_type(MemObjectType::Image2d)
        .dims(&dims)
        .flags(ocl::flags::MEM_WRITE_ONLY | ocl::flags::MEM_HOST_READ_ONLY | ocl::flags::MEM_COPY_HOST_PTR)
        .queue(queue.clone())
        .host_data(&result_image)
        .build().unwrap();

    for frame in 1..100 {
        printlnc!(white_bold: "\nFrame: {}", frame);

        let start_time = time::get_time();

        let cl_source = Image::<u8>::builder()
            .channel_order(ImageChannelOrder::Rgba)
            .channel_data_type(ImageChannelDataType::UnormInt8)
            .image_type(MemObjectType::Image2d)
            .dims(&dims)
            .flags(ocl::flags::MEM_READ_ONLY | ocl::flags::MEM_HOST_WRITE_ONLY | ocl::flags::MEM_COPY_HOST_PTR)
            .queue(queue.clone())
            .host_data(&src_image)
            .build().unwrap();

        print_elapsed("create source", start_time);

        let kernel = Kernel::new("march_penguins", &program).unwrap()
            .queue(queue.clone())
            .gws(&dims)
            .arg_img(&cl_source)
            .arg_img(&cl_dest);

        printlnc!(royal_blue: "Running kernel...");
        printlnc!(white_bold: "image dims: {:?}", &dims);

        kernel.enq().unwrap();
        print_elapsed("kernel enqueued", start_time);

        queue.finish().unwrap();
        print_elapsed("queue finished", start_time);

        cl_dest.read(&mut result_image).enq().unwrap();
        print_elapsed("read finished", start_time);

        // src_image.copy_from(&result_image, 0, 0);
        src_image = result_image.clone();
        print_elapsed("copy", start_time);
    }

    result_image.save(&Path::new("result.png")).unwrap();
}

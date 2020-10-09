// pathfinder/examples/swf_basic/src/main.rs
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use glutin::dpi::LogicalSize;
use glutin::event::{Event, KeyboardInput, VirtualKeyCode, WindowEvent};
use glutin::event_loop::{ControlFlow, EventLoop};
use glutin::window::WindowBuilder;
use glutin::{ContextBuilder, GlProfile, GlRequest};
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::transform2d::Transform2F;
use pathfinder_geometry::vector::{vec2f, vec2i, Vector2F};
use pathfinder_gl::{GLDevice, GLVersion};
use pathfinder_renderer::concurrent::rayon::RayonExecutor;
use pathfinder_renderer::concurrent::scene_proxy::SceneProxy;
use pathfinder_renderer::gpu::options::{DestFramebuffer, RendererMode, RendererOptions};
use pathfinder_renderer::gpu::renderer::Renderer;
use pathfinder_renderer::options::{BuildOptions, RenderTransform};
use pathfinder_renderer::scene::Scene;
use pathfinder_resources::embedded::EmbeddedResourceLoader;
use pathfinder_resources::ResourceLoader;
use pathfinder_swf::{draw_paths_into_scene, process_swf_tags};
use std::env;
use std::fs::read;

fn main() {
    let resource_loader = EmbeddedResourceLoader;

    let swf_bytes;
    if let Some(path) = env::args().skip(1).next() {
        match read(path) {
            Ok(bytes) => {
                swf_bytes = bytes;
            }
            Err(e) => panic!(e),
        }
    } else {
        // NOTE(jon): This is a version of the ghostscript tiger graphic flattened to a single
        // layer with no overlapping shapes.  This is how artwork is 'natively' created in the Flash
        // authoring tool when an artist just draws directly onto the canvas (without 'object' mode
        // turned on, which is the default).
        // Subsequent shapes with different fills will knock out existing fills where they overlap.
        // A downside of this in current pathfinder is that cracks are visible between shape fills -
        // especially obvious if you set the context clear color to #ff00ff or similar.

        // Common speculation as to why the swf format stores vector graphics in this way says that
        // it is to save on file-size bytes, however in the case of our tiger, it results in a
        // larger file than the layered version, since the overlapping shapes and strokes create
        // a lot more geometry.  I think a more likely explanation for the choice is that it was
        // done to reduce overdraw in the software rasterizer running on late 90's era hardware?
        // Indeed, this mode gives pathfinders' occlusion culling pass nothing to do!

        // NOTE(jon): This is a version of the same graphic cut and pasted into the Flash authoring
        // tool from the SVG version loaded in Illustrator. When layered graphics are pasted
        // into Flash, by default they retain their layering, expressed as groups.
        // They are still presented as being on a single timeline layer.
        // They will be drawn back to front in much the same way as the SVG version.

        let default_tiger = resource_loader.slurp("swf/tiger.swf").unwrap();
        swf_bytes = Vec::from(&default_tiger[..]);
    }

    let (_, movie): (_, swf_types::Movie) =
        swf_parser::streaming::movie::parse_movie(&swf_bytes[..]).unwrap();

    // process swf scene
    // TODO(jon): Since swf is a streaming format, this really wants to be a lazy iterator over
    // swf frames eventually.
    let (library, stage) = process_swf_tags(&movie);

    // Calculate the right logical size of the window.
    let event_loop = EventLoop::new();
    let window_size = vec2i(stage.width(), stage.height());
    let logical_window_size = LogicalSize::new(window_size.x(), window_size.y());

    // Open a window.
    let window_builder = WindowBuilder::new()
        .with_title("Minimal example")
        .with_inner_size(logical_window_size);

    // Create an OpenGL 3.x context for Pathfinder to use.
    let gl_context = ContextBuilder::new()
        .with_gl(GlRequest::Latest)
        .with_gl_profile(GlProfile::Core)
        .build_windowed(window_builder, &event_loop)
        .unwrap();

    // Load OpenGL, and make the context current.
    let gl_context = unsafe { gl_context.make_current().unwrap() };
    gl::load_with(|name| gl_context.get_proc_address(name) as *const _);

    let physical_size = gl_context.window().inner_size();

    // Create a Pathfinder renderer.
    let device = GLDevice::new(GLVersion::GL3, 0);
    let mode = RendererMode::default_for_device(&device);
    let options = RendererOptions {
        background_color: Some(stage.background_color()),
        dest: DestFramebuffer::full_window(vec2i(
            physical_size.width as i32,
            physical_size.height as i32,
        )),
        ..RendererOptions::default()
    };
    let mut renderer = Renderer::new(device, &EmbeddedResourceLoader, mode, options);

    let device_pixel_ratio = physical_size.width as f32 / stage.width() as f32;

    // Clear to swf stage background color.
    let mut scene = Scene::new();
    scene.set_view_box(RectF::new(
        Vector2F::zero(),
        vec2f(stage.width() as f32, stage.height() as f32) * device_pixel_ratio,
    ));
    draw_paths_into_scene(&library, &mut scene);

    // Render the canvas to screen.
    let mut scene = SceneProxy::from_scene(scene, renderer.mode().level, RayonExecutor);
    let mut build_options = BuildOptions::default();
    let scale_transform = Transform2F::from_scale(device_pixel_ratio);
    build_options.transform = RenderTransform::Transform2D(scale_transform);
    scene.build_and_render(&mut renderer, build_options.clone());
    println!("{:?}", renderer.last_rendering_time());

    gl_context.swap_buffers().unwrap();
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Poll;

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            }
            | Event::WindowEvent {
                event:
                    WindowEvent::KeyboardInput {
                        input:
                            KeyboardInput {
                                virtual_keycode: Some(VirtualKeyCode::Escape),
                                ..
                            },
                        ..
                    },
                ..
            } => {
                *control_flow = ControlFlow::Exit;
            }
            Event::WindowEvent {
                event: WindowEvent::Resized(physical_size),
                ..
            } => {
                let device_pixel_ratio = physical_size.width as f32 / stage.width() as f32;

                gl_context.resize(physical_size);

                renderer.options_mut().dest = DestFramebuffer::full_window(vec2i(
                    physical_size.width as i32,
                    physical_size.height as i32,
                ));
                renderer.dest_framebuffer_size_changed();

                let scale_transform = Transform2F::from_scale(device_pixel_ratio);
                build_options.transform = RenderTransform::Transform2D(scale_transform);
                scene.set_view_box(RectF::new(
                    Vector2F::zero(),
                    vec2f(stage.width() as f32, stage.height() as f32) * device_pixel_ratio,
                ));
            }
            Event::RedrawRequested(_) => {
                scene.build_and_render(&mut renderer, build_options.clone());
                println!("{:?}", renderer.last_rendering_time());

                gl_context.swap_buffers().unwrap();
            }
            _ => {}
        }
    });
}

// Simple test: Can we create a window with winit?
use winit::event::{Event, WindowEvent};
use winit::event_loop::EventLoop;
use winit::window::WindowBuilder;

fn main() {
    println!("Creating window...");
    
    let event_loop = EventLoop::new().unwrap();
    let window = WindowBuilder::new()
        .with_title("Test Window")
        .with_inner_size(winit::dpi::LogicalSize::new(800, 600))
        .build(&event_loop)
        .unwrap();
    
    println!("Window created! Title: '{}'", window.title());
    println!("Window should now be visible. Press Ctrl+C to exit.");
    
    event_loop.run(|event, elwt| {
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                println!("Window close requested");
                elwt.exit();
            }
            Event::WindowEvent {
                event: WindowEvent::RedrawRequested,
                ..
            } => {
                // Just request another redraw
                window.request_redraw();
            }
            _ => {}
        }
    }).unwrap();
}

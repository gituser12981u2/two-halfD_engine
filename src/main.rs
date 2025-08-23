use std::collections::HashSet;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::camera::Camera;
use crate::world::{Sector, Wall, World};

mod camera;
mod renderer;
mod world;

struct App {
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    world: World,
    camera: Camera,

    keys_down: HashSet<KeyCode>,
    last_tick: Instant,
    move_speed: f32,
    turn_speed: f32,
}

impl Default for App {
    fn default() -> Self {
        let sector = Sector {
            floor_z: 0.0,
            ceiling_z: 3.0,
        };
        let walls = vec![
            Wall {
                start: [-1.0, 8.0],
                end: [1.0, 8.0],
                front_sector: 0,
                back_sector: None,
            },
            Wall {
                start: [1.0, 8.0],
                end: [1.0, 10.0],
                front_sector: 0,
                back_sector: None,
            },
            Wall {
                start: [1.0, 10.0],
                end: [-1.0, 10.0],
                front_sector: 0,
                back_sector: None,
            },
            Wall {
                start: [-1.0, 10.0],
                end: [-1.0, 8.0],
                front_sector: 0,
                back_sector: None,
            },
        ];

        Self {
            window: None,
            surface: None,
            world: World {
                sectors: vec![sector],
                walls,
            },
            camera: Camera {
                pos: [0.0, 0.0],
                yaw: 0.0,   // facing along +Y axis
                eye_z: 1.7, // eye height
                fx: 0.0,
                fy: 0.0,
            },
            keys_down: HashSet::new(),
            last_tick: Instant::now(),
            move_speed: 3.0,                  // m/s
            turn_speed: std::f32::consts::PI, // rad/s
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attributes = Window::default_attributes()
            .with_title("2.5D Engine")
            .with_inner_size(LogicalSize::new(800.0, 600.0));

        let window = Rc::new(event_loop.create_window(attributes).expect("create window"));

        let context = softbuffer::Context::new(window.clone()).expect("softbuffer context");
        let surface =
            softbuffer::Surface::new(&context, window.clone()).expect("softbuffer surface");

        // Update camera focal factors
        let size = window.inner_size();
        self.camera
            .set_fov_from_height(size.width as f32, size.height as f32, 75.0);

        self.surface = Some(surface);
        self.window = Some(window);

        self.last_tick = Instant::now();
        self.window.as_ref().unwrap().request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                println!("The close button was pressed; stopping");
                event_loop.exit();
            }

            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key,
                        state,
                        ..
                    },
                ..
            } => {
                if let PhysicalKey::Code(code) = physical_key {
                    use winit::event::ElementState;
                    match state {
                        ElementState::Pressed => {
                            self.keys_down.insert(code);
                        }
                        ElementState::Released => {
                            self.keys_down.remove(&code);
                        }
                    }
                }
            }

            WindowEvent::RedrawRequested => {
                self.tick();

                let (window, surface) = match (&self.window, &mut self.surface) {
                    (Some(w), Some(s)) if w.id() == id => (w, s),
                    _ => return,
                };

                let size = window.inner_size();
                if size.width == 0 || size.height == 0 {
                    return; // Minimized window, skip drawing
                }

                // Set softbuffer to window size
                surface
                    .resize(
                        NonZeroU32::new(size.width).unwrap(),
                        NonZeroU32::new(size.height).unwrap(),
                    )
                    .unwrap();

                let mut buf = surface.buffer_mut().expect("buffer_mut");
                renderer::render_frame(
                    &mut buf,
                    size.width as usize,
                    size.height as usize,
                    &self.world,
                    &self.camera,
                );

                buf.present().unwrap();
                self.window.as_ref().unwrap().request_redraw();
            }

            WindowEvent::Resized(new_size) => {
                // Update camera focal factors
                self.camera.set_fov_from_height(
                    new_size.width as f32,
                    new_size.height as f32,
                    75.0,
                );
            }
            _ => (),
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

impl App {
    fn tick(&mut self) {
        // Compute dt (cap to avoid huge jumps if the app was paused)
        let now = Instant::now();
        let mut dt = now.duration_since(self.last_tick);
        self.last_tick = now;
        if dt > Duration::from_millis(100) {
            dt = Duration::from_millis(100);
        }
        let dt_s = dt.as_secs_f32();

        // Build movement vector in camera space
        let mut fwd = 0.0;
        let mut strafe = 0.0;
        if self.keys_down.contains(&KeyCode::KeyW) {
            fwd += 1.0;
        }
        if self.keys_down.contains(&KeyCode::KeyS) {
            fwd -= 1.0;
        }
        if self.keys_down.contains(&KeyCode::KeyD) {
            strafe += 1.0;
        }
        if self.keys_down.contains(&KeyCode::KeyA) {
            strafe -= 1.0;
        }

        // Normalize diagonal speed
        if fwd != 0.0 || strafe != 0.0 {
            let len = ((fwd * fwd + strafe * strafe) as f32).sqrt();
            let inv = 1.0 / len;
            fwd *= inv;
            strafe *= inv;
        }

        // Turn with Q/E
        let mut yaw_delta = 0.0;
        if self.keys_down.contains(&KeyCode::KeyQ) {
            yaw_delta -= 1.0;
        }
        if self.keys_down.contains(&KeyCode::KeyE) {
            yaw_delta += 1.0;
        }

        // Apply yaw
        self.camera.yaw += yaw_delta * self.turn_speed * dt_s;
        // Keep yaw in [-pi, pi] to avoid float drift
        if self.camera.yaw > std::f32::consts::PI {
            self.camera.yaw -= 2.0 * std::f32::consts::PI;
        }
        if self.camera.yaw < -std::f32::consts::PI {
            self.camera.yaw += 2.0 * std::f32::consts::PI;
        }

        // Move in world space based on yaw
        if fwd != 0.0 || strafe != 0.0 {
            let c = self.camera.yaw.cos();
            let s = self.camera.yaw.sin();
            // forward vector (0, +1) rotated by yaw = (s, c) in our +Y-forward convention
            let dir_fwd = [s, c];
            let dir_right = [c, -s]; // perpendicular (right-hand)

            let speed = self.move_speed;
            let dx = (dir_fwd[0] * fwd + dir_right[0] * strafe) * speed * dt_s;
            let dy = (dir_fwd[1] * fwd + dir_right[1] * strafe) * speed * dt_s;

            self.camera.pos[0] += dx;
            self.camera.pos[1] += dy;
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().unwrap();

    // ControlFlow::Poll continuously runs the event loop, even if the OS hasn't
    // dispatched any events. This is ideal for games and similar applications.
    event_loop.set_control_flow(ControlFlow::Poll);

    // ControlFlow::Wait pauses the event loop if no events are available to process.
    // This is ideal for non-game applications that only update in response to user
    // input, and uses significantly less power/CPU time than ControlFlow::Poll.
    // event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = App::default();
    event_loop.run_app(&mut app);
}

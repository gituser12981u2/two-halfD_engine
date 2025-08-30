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
use crate::scaler::{ScaleLut, blit_bilinear_stretch, build_scale_lut, sharpen3x3_cross_inplace};
use crate::world::{Sector, Wall, World};

mod camera;
mod renderer;
mod scaler;
mod world;

struct App {
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    world: World,
    camera: Camera,

    // HUD
    frame_counter: u32,
    last_fps_print: Instant,

    // Internal 640x480 buffer
    fb_small: Vec<u32>,
    fb_w: usize,
    fb_h: usize,

    scale_lut: ScaleLut,

    // Input and movement
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

            frame_counter: 0,
            last_fps_print: Instant::now(),

            fb_small: vec![0; 640 * 480],
            fb_w: 640,
            fb_h: 480,

            scale_lut: ScaleLut::empty(),

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
        self.rebuild_internal_fb_and_lut(size.width as usize, size.height as usize);

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
                let (dw, dh) = (size.width as usize, size.height as usize);
                if dw == 0 || dh == 0 {
                    return; // Minimized window, skip drawing
                }

                // Set softbuffer to window size
                surface
                    .resize(
                        NonZeroU32::new(dw as u32).unwrap(),
                        NonZeroU32::new(dh as u32).unwrap(),
                    )
                    .unwrap();

                renderer::render_frame(
                    &mut self.fb_small,
                    self.fb_w,
                    self.fb_h,
                    &self.world,
                    &self.camera,
                );

                let mut buf = surface.buffer_mut().expect("buffer_mut");
                blit_bilinear_stretch(&mut buf, dw, &self.fb_small, self.fb_w, &self.scale_lut);

                sharpen3x3_cross_inplace(&mut buf, dw, dh);

                buf.present().unwrap();

                // Print FPS
                self.frame_counter += 1;
                let now = Instant::now();
                if now.duration_since(self.last_fps_print).as_secs_f32() >= 1.0 {
                    let fps = self.frame_counter as f32
                        / now.duration_since(self.last_fps_print).as_secs_f32();
                    println!("FPS: {:.1}", fps);
                    self.frame_counter = 0;
                    self.last_fps_print = now;
                }

                self.window.as_ref().unwrap().request_redraw();
            }

            WindowEvent::Resized(new_size) => {
                let (dw, dh) = (new_size.width as usize, new_size.height as usize);
                // Update internal window
                self.rebuild_internal_fb_and_lut(dw, dh);
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
        // Compute dt with cap to avoid huge jumps if the app was paused
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
            // forward vector (0, +1) rotated by yaw = (s, c) in +Y forward convention
            let dir_fwd = [s, c];
            let dir_right = [c, -s]; // perpendicular (right-hand)

            let speed = self.move_speed;
            let dx = (dir_fwd[0] * fwd + dir_right[0] * strafe) * speed * dt_s;
            let dy = (dir_fwd[1] * fwd + dir_right[1] * strafe) * speed * dt_s;

            self.camera.pos[0] += dx;
            self.camera.pos[1] += dy;
        }
    }

    fn rebuild_internal_fb_and_lut(&mut self, dst_w: usize, dst_h: usize) {
        // Keep internal height fixed (controls pixel size look)
        let target_h = 480usize;
        let aspect = if dst_h > 0 {
            dst_w as f32 / dst_h as f32
        } else {
            1.0
        };

        // Derive width from aspect
        let mut target_w = (target_h as f32 * aspect).round() as usize; // round to even for SIMD alignment
        if target_w < 160 {
            target_w = 160;
        }
        if target_w % 2 != 0 {
            target_w += 1;
        }

        // Reallocate internal FB if size changed
        if target_w != self.fb_w || target_h != self.fb_h {
            self.fb_w = target_w;
            self.fb_h = target_h;
            self.fb_small = vec![0u32; self.fb_w * self.fb_h];
        }

        self.camera
            .set_fov_from_horizontal(self.fb_w as f32, self.fb_h as f32, 90.0);
        self.scale_lut = build_scale_lut(dst_w, dst_h, self.fb_w, self.fb_h);
    }
}

fn main() {
    let event_loop = EventLoop::new().unwrap();

    // ControlFlow::Poll continuously runs the event loop, even if the OS hasn't
    // dispatched any events. This is ideal for games and similar applications.
    // event_loop.set_control_flow(ControlFlow::Poll);

    // ControlFlow::Wait pauses the event loop if no events are available to process.
    // This is ideal for non-game applications that only update in response to user
    // input, and uses significantly less power/CPU time than ControlFlow::Poll.
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = App::default();
    let _ = event_loop.run_app(&mut app);
}

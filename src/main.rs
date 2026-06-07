mod metrics;
mod temperature;

use std::time::{Duration, Instant};

use muda::{Menu, MenuEvent, MenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

#[cfg(target_os = "macos")]
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};

const POLL_INTERVAL: Duration = Duration::from_secs(3);

struct App {
    tray: Option<TrayIcon>,
    quit_id: Option<muda::MenuId>,
    last_poll: Instant,
}

impl App {
    fn new() -> Self {
        Self {
            tray: None,
            quit_id: None,
            last_poll: Instant::now() - POLL_INTERVAL,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        if self.tray.is_some() {
            return;
        }
        let quit = MenuItem::new("Quit stamon", true, None);
        self.quit_id = Some(quit.id().clone());
        let menu = Menu::new();
        menu.append(&quit).expect("failed to append menu item");
        self.tray = Some(
            TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .build()
                .expect("failed to create tray icon"),
        );
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.last_poll.elapsed() >= POLL_INTERVAL {
            if let Some(tray) = &mut self.tray {
                let temps = temperature::read_temps();
                let m = metrics::read_metrics();
                let cpu_temp = temps.cpu.map_or("--".into(), |t| format!("{:.0}°", t));
                let gpu_temp = temps.gpu.map_or("--".into(), |t| format!("{:.0}°", t));
                let cpu_pct = m.cpu_pct.map_or("--".into(), |p| format!("{:.0}%", p));
                let ram = format!("{:.1}/{:.0}G", m.ram_used_gb, m.ram_total_gb);
                tray.set_title(Some(format!(
                    "CPU {} {}  GPU {}  RAM {}",
                    cpu_temp, cpu_pct, gpu_temp, ram
                )));
            }
            self.last_poll = Instant::now();
        }

        if let Ok(ev) = MenuEvent::receiver().try_recv() {
            if self.quit_id.as_ref() == Some(&ev.id) {
                event_loop.exit();
            }
        }

        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + POLL_INTERVAL));
    }
}

fn main() {
    #[cfg(target_os = "macos")]
    let event_loop = EventLoop::builder()
        .with_activation_policy(ActivationPolicy::Accessory)
        .build()
        .expect("failed to create event loop");

    #[cfg(not(target_os = "macos"))]
    let event_loop = EventLoop::new().expect("failed to create event loop");

    event_loop
        .run_app(&mut App::new())
        .expect("event loop error");
}

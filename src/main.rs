mod metrics;
mod statusbar;
mod temperature;

use std::time::{Duration, Instant};

use muda::{accelerator::Accelerator, CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use statusbar::{BoxSpec, StatusBar};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

#[cfg(target_os = "macos")]
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};

const POLL_INTERVAL: Duration = Duration::from_secs(5);

const T_CPU_TEMP: usize = 0;
const T_GPU_TEMP: usize = 1;
const T_CPU_USE:  usize = 2;
const T_GPU_USE:  usize = 3;
const T_RAM:      usize = 4;

struct App {
    statusbar: Option<StatusBar>,
    menu:      Option<Menu>,
    quit_id:   Option<muda::MenuId>,
    toggles:   [Option<CheckMenuItem>; 5],
    last_poll:  Instant,
}

impl App {
    fn new() -> Self {
        Self {
            statusbar: None,
            menu:      None,
            quit_id:   None,
            toggles:   [None, None, None, None, None],
            last_poll:  Instant::now() - POLL_INTERVAL,
        }
    }

    fn on(&self, idx: usize) -> bool {
        self.toggles[idx].as_ref().map_or(true, |t| t.is_checked())
    }

    fn is_toggle(&self, id: &muda::MenuId) -> bool {
        self.toggles.iter().flatten().any(|t| t.id() == id)
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        if self.statusbar.is_some() {
            return;
        }

        let check = |label| CheckMenuItem::new(label, true, true, None::<Accelerator>);
        let cpu_temp = check("CPU Temperature");
        let gpu_temp = check("GPU Temperature");
        let cpu_use  = check("CPU Usage");
        let gpu_use  = check("GPU Usage");
        let ram      = check("RAM Usage");

        let quit = MenuItem::new("Quit Vapor", true, None::<Accelerator>);
        self.quit_id = Some(quit.id().clone());

        let menu = Menu::new();
        menu.append(&cpu_temp).unwrap();
        menu.append(&gpu_temp).unwrap();
        menu.append(&cpu_use).unwrap();
        menu.append(&gpu_use).unwrap();
        menu.append(&ram).unwrap();
        menu.append(&PredefinedMenuItem::separator()).unwrap();
        menu.append(&quit).unwrap();

        self.toggles = [
            Some(cpu_temp),
            Some(gpu_temp),
            Some(cpu_use),
            Some(gpu_use),
            Some(ram),
        ];

        self.statusbar = Some(StatusBar::new(&menu));
        self.menu      = Some(menu);
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Ok(ev) = MenuEvent::receiver().try_recv() {
            if self.quit_id.as_ref() == Some(&ev.id) {
                event_loop.exit();
                return;
            }
            if self.is_toggle(&ev.id) {
                self.last_poll = Instant::now() - POLL_INTERVAL;
            }
        }

        if self.last_poll.elapsed() >= POLL_INTERVAL {
            let want_cpu_temp = self.on(T_CPU_TEMP);
            let want_gpu_temp = self.on(T_GPU_TEMP);
            let want_cpu_use  = self.on(T_CPU_USE);
            let want_gpu_use  = self.on(T_GPU_USE);
            let want_ram      = self.on(T_RAM);

            let temps = temperature::read_temps(want_cpu_temp, want_gpu_temp);
            let m = metrics::read_metrics(metrics::MetricsFlags {
                cpu_usage: want_cpu_use,
                gpu_usage: want_gpu_use,
                ram:       want_ram,
            });

            let mut boxes: Vec<BoxSpec<'_>> = Vec::new();

            let cpu_val = (want_cpu_temp || want_cpu_use).then(|| match (want_cpu_temp, want_cpu_use) {
                (true,  true)  => format!(
                    "{}/{}",
                    temps.cpu.map_or("--".into(), |t| format!("{:.0}°", t)),
                    m.cpu_pct.map_or("--".into(), |p| format!("{:.0}%", p)),
                ),
                (true,  false) => temps.cpu.map_or("--".into(), |t| format!("{:.0}°", t)),
                (false, true)  => m.cpu_pct.map_or("--".into(), |p| format!("{:.0}%", p)),
                _              => unreachable!(),
            });
            if let Some(ref v) = cpu_val { boxes.push(BoxSpec { label: "CPU", value: v }); }

            let gpu_val = (want_gpu_temp || want_gpu_use).then(|| match (want_gpu_temp, want_gpu_use) {
                (true,  true)  => format!(
                    "{}/{}",
                    temps.gpu.map_or("--".into(), |t| format!("{:.0}°", t)),
                    m.gpu_pct.map_or("--".into(), |p| format!("{:.0}%", p)),
                ),
                (true,  false) => temps.gpu.map_or("--".into(), |t| format!("{:.0}°", t)),
                (false, true)  => m.gpu_pct.map_or("--".into(), |p| format!("{:.0}%", p)),
                _              => unreachable!(),
            });
            if let Some(ref v) = gpu_val { boxes.push(BoxSpec { label: "GPU", value: v }); }

            let ram_val = want_ram
                .then(|| format!("{:.1}/{:.0}G", m.ram_used_gb, m.ram_total_gb));
            if let Some(ref v) = ram_val { boxes.push(BoxSpec { label: "RAM", value: v }); }

            if let Some(sb) = &self.statusbar {
                sb.update(&boxes);
            }

            self.last_poll = Instant::now();
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

    event_loop.run_app(&mut App::new()).expect("event loop error");
}

// A Slint front end for winget: list upgradable packages, choose some, and run
// the upgrade in a terminal you can watch; pin the ones you would rather leave
// alone.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod winget;

use std::cell::RefCell;
use std::collections::HashSet;

use slint::{ComponentHandle, ModelRc, VecModel};

slint::include_modules!();

/// Everything the window is a view of. Lives on the UI thread only: winget calls
/// happen on worker threads and hand their results back through
/// `upgrade_in_event_loop`, which runs on this thread, so a thread-local needs no
/// locking and keeps the callbacks free of shared-ownership plumbing.
#[derive(Default)]
struct State {
    upgrades: Vec<winget::Upgrade>,
    pins: Vec<winget::Pin>,
    /// Package identifiers ticked for upgrade. Kept by identifier rather than by
    /// row index so a rescan does not silently move the selection to other rows.
    selected: HashSet<String>,
    filter: String,
}

impl State {
    /// Upgrades matching the current filter, in display order.
    fn visible(&self) -> Vec<&winget::Upgrade> {
        let f = self.filter.to_lowercase();
        self.upgrades
            .iter()
            .filter(|u| {
                f.is_empty()
                    || u.name.to_lowercase().contains(&f)
                    || u.id.to_lowercase().contains(&f)
            })
            .collect()
    }

    /// Selected identifiers that are still on offer, in display order, so the
    /// command matches what the user can see ticked.
    fn selected_ids(&self) -> Vec<String> {
        self.upgrades
            .iter()
            .filter(|u| self.selected.contains(&u.id))
            .map(|u| u.id.clone())
            .collect()
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;

    ui.on_refresh({
        let ui = ui.as_weak();
        move || scan(ui.clone())
    });

    ui.on_toggle({
        let ui = ui.as_weak();
        move |id| {
            let id = id.to_string();
            with_state(|s| {
                if !s.selected.remove(&id) {
                    s.selected.insert(id);
                }
            });
            if let Some(ui) = ui.upgrade() {
                render(&ui);
            }
        }
    });

    // Ticks every visible row, or clears them all if they are already ticked.
    // Rows hidden by the filter are left alone either way.
    ui.on_select_all({
        let ui = ui.as_weak();
        move || {
            with_state(|s| {
                let visible: Vec<String> = s.visible().iter().map(|u| u.id.clone()).collect();
                if visible.iter().all(|id| s.selected.contains(id)) {
                    for id in visible {
                        s.selected.remove(&id);
                    }
                } else {
                    for id in visible {
                        s.selected.insert(id);
                    }
                }
            });
            if let Some(ui) = ui.upgrade() {
                render(&ui);
            }
        }
    });

    ui.on_filter_changed({
        let ui = ui.as_weak();
        move |text| {
            with_state(|s| s.filter = text.trim().to_string());
            if let Some(ui) = ui.upgrade() {
                render(&ui);
            }
        }
    });

    ui.on_add_pin({
        let ui = ui.as_weak();
        move |id, blocking| {
            let id = id.to_string();
            mutate(ui.clone(), move || winget::add_pin(&id, blocking));
        }
    });

    ui.on_remove_pin({
        let ui = ui.as_weak();
        move |id| {
            let id = id.to_string();
            mutate(ui.clone(), move || winget::remove_pin(&id));
        }
    });

    ui.on_run_upgrade({
        let ui = ui.as_weak();
        move || {
            let ids = with_state(|s| s.selected_ids());
            let Some(window) = ui.upgrade() else { return };

            let run = match winget::launch_upgrade(&ids) {
                Ok(run) => run,
                Err(e) => return window.set_message(e.to_string().into()),
            };

            // winget serialises: a second command blocks on its own lock while an
            // install is in flight. So the whole list goes read-only rather than
            // letting a pin or a rescan queue up behind the upgrade and appear to
            // hang.
            window.set_upgrading(true);
            window.set_message("Upgrading in a terminal window.".into());

            let weak = ui.clone();
            std::thread::spawn(move || {
                let outcome = run.wait();
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    ui.set_upgrading(false);
                    let note = match outcome {
                        winget::UpgradeOutcome::Finished(0) => {
                            "Upgrade finished.".to_string()
                        }
                        winget::UpgradeOutcome::Finished(code) => {
                            format!("winget exited with code {code}. The terminal has the detail.")
                        }
                        winget::UpgradeOutcome::TerminalClosed => {
                            "The terminal closed before winget reported.".to_string()
                        }
                    };
                    scan_noting(ui.as_weak(), note);
                });
            });
        }
    });

    scan(ui.as_weak());
    ui.run()
}

fn scan(weak: slint::Weak<MainWindow>) {
    scan_noting(weak, String::new());
}

/// Read the client version, the upgrade list and the pin list on a worker
/// thread, then apply all three at once.
///
/// `note` is shown while the scan runs and again when it finds nothing wrong, so
/// a message set by whatever triggered the scan survives it. A real problem still
/// wins: this is the place errors surface.
fn scan_noting(weak: slint::Weak<MainWindow>, note: String) {
    let Some(ui) = weak.upgrade() else { return };
    ui.set_busy(true);
    ui.set_message(note.as_str().into());

    std::thread::spawn(move || {
        let version = winget::version();
        let upgrades = winget::list_upgrades();
        let pins = winget::list_pins();

        let _ = weak.upgrade_in_event_loop(move |ui| {
            let mut problem = None;

            match upgrades {
                Ok(v) => with_state(|s| {
                    // Drop selections for packages that are no longer offered,
                    // so the count cannot outrun the list.
                    let live: HashSet<&String> = v.iter().map(|u| &u.id).collect();
                    s.selected.retain(|id| live.contains(id));
                    s.upgrades = v;
                }),
                Err(e) => problem = Some(e.to_string()),
            }
            match pins {
                Ok(v) => with_state(|s| s.pins = v),
                Err(e) => problem = problem.or(Some(e.to_string())),
            }

            if let Ok(v) = version {
                ui.set_client_version(v.trim_start_matches('v').into());
            }
            ui.set_scanned_at(clock().into());
            ui.set_busy(false);
            ui.set_message(problem.unwrap_or(note).into());
            render(&ui);
        });
    });
}

/// Run a winget command that changes pin state, then rescan so the lists reflect
/// what winget actually did rather than what was asked for.
fn mutate(
    weak: slint::Weak<MainWindow>,
    action: impl FnOnce() -> winget::Result<()> + Send + 'static,
) {
    let Some(ui) = weak.upgrade() else { return };
    ui.set_busy(true);

    std::thread::spawn(move || {
        let result = action();
        let _ = weak.upgrade_in_event_loop(move |ui| match result {
            Ok(()) => scan(ui.as_weak()),
            Err(e) => {
                ui.set_busy(false);
                ui.set_message(e.to_string().into());
            }
        });
    });
}

/// Push the whole of `State` into the window's properties.
fn render(ui: &MainWindow) {
    with_state(|s| {
        let rows: Vec<PackageRow> = s
            .visible()
            .iter()
            .map(|u| PackageRow {
                name: u.name.as_str().into(),
                id: u.id.as_str().into(),
                current: u.current.as_str().into(),
                available: u.available.as_str().into(),
                selected: s.selected.contains(&u.id),
            })
            .collect();

        let pins: Vec<PinRow> = s
            .pins
            .iter()
            .map(|p| PinRow {
                name: p.name.as_str().into(),
                id: p.id.as_str().into(),
                kind: p.kind.as_str().into(),
                blocking: p.kind == winget::PinKind::Blocking,
            })
            .collect();

        let selected = s.selected_ids();

        ui.set_upgrade_count(rows.len() as i32);
        ui.set_selected_count(selected.len() as i32);
        ui.set_pinning_count(
            s.pins
                .iter()
                .filter(|p| p.kind != winget::PinKind::Blocking)
                .count() as i32,
        );
        ui.set_blocking_count(
            s.pins
                .iter()
                .filter(|p| p.kind == winget::PinKind::Blocking)
                .count() as i32,
        );
        ui.set_filtering(!s.filter.is_empty());
        ui.set_command_preview(if selected.is_empty() {
            "Select at least one package.".into()
        } else {
            winget::upgrade_command(&selected).into()
        });
        ui.set_upgrades(ModelRc::new(VecModel::from(rows)));
        ui.set_pins(ModelRc::new(VecModel::from(pins)));
    });
}

/// Local wall-clock time as HH:MM:SS. Asking Windows directly avoids pulling in
/// a date crate, and a timezone, for one label.
#[cfg(windows)]
fn clock() -> String {
    #[repr(C)]
    #[derive(Default)]
    struct Systemtime {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }

    extern "system" {
        fn GetLocalTime(out: *mut Systemtime);
    }

    let mut t = Systemtime::default();
    // Safety: GetLocalTime only writes the SYSTEMTIME it is handed.
    unsafe { GetLocalTime(&mut t) };
    format!("{:02}:{:02}:{:02}", t.hour, t.minute, t.second)
}

#[cfg(not(windows))]
fn clock() -> String {
    use std::time::SystemTime;
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        % 86_400;
    format!("{:02}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
}

//! Serveur IPC : socket Unix, une requête JSON par ligne (voir `noos_core::ipc`).

use std::{
    collections::HashMap,
    io::{ErrorKind, Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
};

use noos_core::ipc::{Event, Request};
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction, generic::Generic};
use tracing::{debug, info, warn};

use crate::state::NoosState;

struct Client {
    stream: UnixStream,
    subscribed: bool,
}

pub struct IpcServer {
    path: PathBuf,
    clients: HashMap<u64, Client>,
    next_id: u64,
}

impl IpcServer {
    pub fn new(handle: &LoopHandle<'static, NoosState>) -> Self {
        let path = socket_path();
        let _ = std::fs::remove_file(&path);
        match UnixListener::bind(&path) {
            Ok(listener) => {
                listener.set_nonblocking(true).ok();
                handle
                    .insert_source(Generic::new(listener, Interest::READ, Mode::Level), |_, listener, state| {
                        loop {
                            match listener.accept() {
                                Ok((stream, _)) => state.ipc_accept(stream),
                                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                                Err(e) => {
                                    warn!("ipc accept : {e}");
                                    break;
                                }
                            }
                        }
                        Ok(PostAction::Continue)
                    })
                    .expect("source ipc");
                info!(path = %path.display(), "socket IPC prêt");
            }
            Err(e) => warn!(path = %path.display(), "IPC indisponible : {e}"),
        }
        Self { path, clients: HashMap::new(), next_id: 1 }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn send_to(&mut self, id: u64, event: &Event) {
        let Some(client) = self.clients.get_mut(&id) else { return };
        let mut line = serde_json::to_string(event).unwrap();
        line.push('\n');
        if client.stream.write_all(line.as_bytes()).is_err() {
            debug!(id, "client IPC perdu");
            self.clients.remove(&id);
        }
    }

    /// Envoie un événement à tous les clients abonnés.
    pub fn broadcast(&mut self, event: &Event) {
        let ids: Vec<u64> = self.clients.iter().filter(|(_, c)| c.subscribed).map(|(id, _)| *id).collect();
        for id in ids {
            self.send_to(id, event);
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn socket_path() -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    runtime.join(format!("noos-ipc-{}.sock", std::process::id()))
}

impl NoosState {
    fn ipc_accept(&mut self, stream: UnixStream) {
        let id = self.ipc.next_id;
        self.ipc.next_id += 1;
        let Ok(writer) = stream.try_clone() else { return };
        stream.set_nonblocking(true).ok();
        self.ipc.clients.insert(id, Client { stream: writer, subscribed: false });

        let mut buffer = Vec::<u8>::new();
        let res = self.loop_handle.insert_source(
            Generic::new(stream, Interest::READ, Mode::Level),
            move |_, stream, state| {
                let mut chunk = [0u8; 4096];
                loop {
                    // SAFETY : on ne remplace pas le flux pendant la lecture.
                    match unsafe { stream.get_mut() }.read(&mut chunk) {
                        Ok(0) => {
                            state.ipc.clients.remove(&id);
                            return Ok(PostAction::Remove);
                        }
                        Ok(n) => buffer.extend_from_slice(&chunk[..n]),
                        Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                        Err(_) => {
                            state.ipc.clients.remove(&id);
                            return Ok(PostAction::Remove);
                        }
                    }
                }
                while let Some(pos) = buffer.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = buffer.drain(..=pos).collect();
                    let text = String::from_utf8_lossy(&line);
                    if text.trim().is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<Request>(&text) {
                        Ok(req) => state.handle_ipc(id, req),
                        Err(e) => state.ipc.send_to(id, &Event::Error { message: format!("requête invalide : {e}") }),
                    }
                }
                if state.ipc.clients.contains_key(&id) { Ok(PostAction::Continue) } else { Ok(PostAction::Remove) }
            },
        );
        if let Err(e) = res {
            warn!("ipc client : {e}");
        }
    }

    fn handle_ipc(&mut self, client: u64, req: Request) {
        debug!(?req, "requête IPC");
        let reply = match req {
            Request::Subscribe => {
                if let Some(c) = self.ipc.clients.get_mut(&client) {
                    c.subscribed = true;
                }
                let outputs = self.backend.output_infos(&self.space, &self.config);
                self.ipc.send_to(client, &Event::Outputs { outputs });
                let desktops = Event::Desktops { names: self.desktop_names(), current: self.desktops.current };
                self.ipc.send_to(client, &desktops);
                Event::Windows { windows: self.window_list() }
            }
            Request::GetWindows => {
                let mut windows = self.window_list();
                for info in windows.iter_mut().filter(|w| !w.minimized) {
                    if let Some(geo) = self.window_by_id(info.id).and_then(|w| self.space.element_geometry(&w)) {
                        info.geometry = Some([geo.loc.x, geo.loc.y, geo.size.w, geo.size.h]);
                    } else if let Some((w, loc)) = self.desktops.parked.iter().find(|(w, _)| crate::state::window_data(w).id == info.id) {
                        // Fenêtre d'un autre bureau : sa position garée (miniatures des bureaux).
                        let size = w.geometry().size;
                        info.geometry = Some([loc.x, loc.y, size.w, size.h]);
                    }
                }
                Event::Windows { windows }
            }
            Request::GetOutputs => Event::Outputs { outputs: self.backend.output_infos(&self.space, &self.config) },
            Request::Activate { id } => self.with_window(id, |s, w| {
                // Clic sur l'icône de la fenêtre déjà active = réduire (comme Windows).
                if s.focused_window().as_ref() == Some(&w) && !s.minimized.contains(&w) {
                    s.minimize(&w)
                } else {
                    s.activate_window(&w)
                }
            }),
            Request::Close { id } => self.with_window(id, |s, w| s.close_window(&w)),
            Request::Minimize { id } => self.with_window(id, |s, w| s.minimize(&w)),
            Request::ToggleMaximize { id } => self.with_window(id, |s, w| s.toggle_maximize(&w)),
            Request::Snap { id, zone } => {
                use crate::tiling::Zone::*;
                let z = match zone.as_str() {
                    "maximize" => Some(Maximize),
                    "left" => Some(Left),
                    "right" => Some(Right),
                    "top-left" => Some(TopLeft),
                    "top-right" => Some(TopRight),
                    "bottom-left" => Some(BottomLeft),
                    "bottom-right" => Some(BottomRight),
                    "left-third" => Some(LeftThird),
                    "center-third" => Some(CenterThird),
                    "right-third" => Some(RightThird),
                    "left-two-thirds" => Some(LeftTwoThirds),
                    "right-two-thirds" => Some(RightTwoThirds),
                    _ => None,
                };
                self.with_window(id, |s, w| match z {
                    Some(z) => s.snap_zone(&w, z),
                    None => s.unmaximize(&w, None),
                })
            }
            Request::ShowDesktop => {
                self.show_desktop();
                Event::Ok
            }
            Request::ReloadConfig => {
                self.reload_config();
                Event::Ok
            }
            Request::Spawn { command } => {
                self.spawn(&command);
                Event::Ok
            }
            Request::SetMode { output, width, height, refresh } => match self.backend.set_mode(&output, width, height, refresh) {
                Ok(o) => {
                    self.output_changed(&o);
                    Event::Ok
                }
                Err(message) => Event::Error { message },
            },
            Request::SetScale { output, scale } => match self.find_output(&output) {
                Some(o) if (0.5..=4.0).contains(&scale) => {
                    o.change_current_state(None, None, Some(smithay::output::Scale::Fractional(scale)), None);
                    self.output_changed(&o);
                    Event::Ok
                }
                Some(_) => Event::Error { message: "échelle hors limites (0,5 – 4)".into() },
                None => Event::Error { message: format!("écran {output} inconnu") },
            },
            Request::SetTransform { output, transform } => {
                if !self.backend.supports_transform() {
                    Event::Error { message: "orientation non disponible en mode imbriqué".into() }
                } else if let Some(o) = self.find_output(&output) {
                    o.change_current_state(None, Some(crate::wm::parse_transform(&transform)), None, None);
                    self.output_changed(&o);
                    Event::Ok
                } else {
                    Event::Error { message: format!("écran {output} inconnu") }
                }
            }
            Request::ToggleTaskView => {
                self.ipc.broadcast(&Event::ToggleTaskView);
                Event::Ok
            }
            Request::GetDesktops => Event::Desktops { names: self.desktop_names(), current: self.desktops.current },
            Request::SwitchDesktop { index } => {
                self.switch_desktop(index);
                Event::Ok
            }
            Request::SwitchDesktopRelative { delta } => {
                self.switch_desktop_relative(delta);
                Event::Ok
            }
            Request::AddDesktop { name, switch } => {
                self.add_desktop(name, switch);
                Event::Ok
            }
            Request::RemoveDesktop { index } => {
                self.remove_desktop(index);
                Event::Ok
            }
            Request::RenameDesktop { index, name } => {
                self.rename_desktop(index, name);
                Event::Ok
            }
            Request::MoveDesktop { from, to } => {
                self.move_desktop(from, to);
                Event::Ok
            }
            Request::MoveWindowToDesktop { id, index, follow } => self.with_window(id, |s, w| s.move_window_to_desktop(&w, index, follow)),
            Request::SetSticky { id, sticky } => self.with_window(id, |s, w| s.set_sticky(&w, sticky)),
            Request::MoveWindowToOutput { id, output } => self.with_window(id, |s, w| {
                let target = s.space.outputs().find(|o| o.name() == output).cloned();
                if let Some(o) = target {
                    s.move_window_to_output(&w, &o);
                }
            }),
            Request::ToggleStartMenu => {
                self.ipc.broadcast(&Event::ToggleStartMenu);
                Event::Ok
            }
            Request::Screenshot { path } => {
                self.pending_screenshot = Some(path.into());
                Event::Ok
            }
            Request::Lock => {
                self.lock_session();
                Event::Ok
            }
            Request::Suspend => {
                self.suspend_with_lock();
                Event::Ok
            }
            Request::Quit => {
                self.loop_signal.stop();
                Event::Ok
            }
        };
        self.ipc.send_to(client, &reply);
    }

    fn find_output(&self, name: &str) -> Option<smithay::output::Output> {
        self.space.outputs().find(|o| o.name() == name).cloned()
    }

    fn with_window(&mut self, id: u64, f: impl FnOnce(&mut Self, smithay::desktop::Window)) -> Event {
        match self.window_by_id(id) {
            Some(w) => {
                f(self, w);
                self.notify_windows_changed();
                Event::Ok
            }
            None => Event::Error { message: format!("fenêtre {id} inconnue") },
        }
    }
}

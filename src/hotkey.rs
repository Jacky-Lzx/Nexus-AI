use eframe::egui;
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
};
use std::{sync::mpsc, thread};

pub const SHORTCUT_LABEL: &str = "Cmd+Shift+Space";

pub struct HotkeyController {
    manager: GlobalHotKeyManager,
    hotkey: HotKey,
    events: mpsc::Receiver<()>,
}

impl HotkeyController {
    pub fn register(ctx: egui::Context) -> Result<Self, String> {
        let manager = GlobalHotKeyManager::new().map_err(|error| error.to_string())?;
        let hotkey = HotKey::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::Space);
        manager
            .register(hotkey)
            .map_err(|error| error.to_string())?;

        let hotkey_id = hotkey.id;
        let receiver = GlobalHotKeyEvent::receiver().clone();
        let (sender, events) = mpsc::channel();
        thread::spawn(move || {
            while let Ok(event) = receiver.recv() {
                if event.id == hotkey_id && event.state == HotKeyState::Pressed {
                    if sender.send(()).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            }
        });

        Ok(Self {
            manager,
            hotkey,
            events,
        })
    }

    pub fn take_triggered(&self) -> bool {
        let mut triggered = false;
        while self.events.try_recv().is_ok() {
            triggered = true;
        }
        triggered
    }
}

impl Drop for HotkeyController {
    fn drop(&mut self) {
        let _ = self.manager.unregister(self.hotkey);
    }
}

use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

#[derive(Debug)]
pub enum ClipboardEvent {
    Text(String),
    Unavailable(String),
}

#[derive(Debug)]
pub enum ClipboardCommand {
    SetText(String),
    Stop,
}

pub struct ClipboardWorker {
    pub events: Receiver<ClipboardEvent>,
    commands: Sender<ClipboardCommand>,
}

impl ClipboardWorker {
    pub fn start() -> Self {
        let (event_tx, events) = mpsc::channel();
        let (commands, command_rx) = mpsc::channel();

        thread::spawn(move || run_worker(event_tx, command_rx));
        Self { events, commands }
    }

    pub fn set_text(&self, text: String) {
        let _ = self.commands.send(ClipboardCommand::SetText(text));
    }
}

impl Drop for ClipboardWorker {
    fn drop(&mut self) {
        let _ = self.commands.send(ClipboardCommand::Stop);
    }
}

fn run_worker(events: Sender<ClipboardEvent>, commands: Receiver<ClipboardCommand>) {
    let mut clipboard = match arboard::Clipboard::new() {
        Ok(clipboard) => clipboard,
        Err(error) => {
            let _ = events.send(ClipboardEvent::Unavailable(error.to_string()));
            return;
        }
    };
    let mut last_text: Option<String> = None;

    loop {
        while let Ok(command) = commands.try_recv() {
            match command {
                ClipboardCommand::SetText(text) => {
                    if clipboard.set_text(text.clone()).is_ok() {
                        last_text = Some(text);
                    }
                }
                ClipboardCommand::Stop => return,
            }
        }

        if let Ok(text) = clipboard.get_text()
            && !text.trim().is_empty()
            && last_text.as_ref() != Some(&text)
        {
            last_text = Some(text.clone());
            if events.send(ClipboardEvent::Text(text)).is_err() {
                return;
            }
        }
        thread::sleep(Duration::from_millis(400));
    }
}

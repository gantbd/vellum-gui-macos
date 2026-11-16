//! Opt-in real-window/readback probe. No system input injection, other-window
//! capture or clipboard workflow. This does not qualify physical keys, IME,
//! accessibility, bidi or cross-app clipboard behavior.
use super::{Gui, Message as GuiMessage};
use crate::Command;
use iced::{Element, Subscription, Task, widget::text_editor, window};
use std::{
    fs,
    hash::{Hash, Hasher},
    io::{self, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const ORIGINAL: &str = "cat\r\ncat\r\nGrüße 👩‍💻";
const EDITED: &str = "X\r\nX\r\nGrüße 👩‍💻";
const MAX_PIXELS: usize = 4096 * 4096;
static NEXT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Waiting,
    Initial,
    Selected,
    Edited,
    Restored,
    Terminal,
}

#[derive(Clone, Debug)]
enum Message {
    Opened(window::Id),
    Capture(window::Screenshot),
    ReadbackLost,
    Deadline,
    Interrupted,
}

#[derive(Debug)]
struct Report {
    screenshot: PathBuf,
    frames: u32,
    // Probe initialization through final readback, not process cold start,
    // per-interaction latency, presentation latency or an adopted SLO.
    elapsed: Duration,
}
type Outcome = Arc<Mutex<Option<Result<Report, String>>>>;

struct DrawWait {
    changed_at: Instant,
    id: window::Id,
    sender: iced::futures::channel::oneshot::Sender<window::Id>,
}
#[derive(Clone, Default)]
struct DrawBarrier(Arc<Mutex<Option<DrawWait>>>);
impl Hash for DrawBarrier {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state)
    }
}
impl DrawBarrier {
    fn observe(&self, at: Instant) {
        let ready = {
            let mut pending = self.0.lock().expect("draw barrier mutex poisoned");
            if pending.as_ref().is_some_and(|wait| at >= wait.changed_at) {
                pending.take()
            } else {
                None
            }
        };
        if let Some(wait) = ready {
            let _ = wait.sender.send(wait.id);
        }
    }
}

struct Fixture {
    directory: PathBuf,
    source: PathBuf,
}
impl Fixture {
    fn new() -> io::Result<Self> {
        loop {
            let directory = std::env::temp_dir().join(format!(
                "vellum-live-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&directory) {
                Ok(()) => {
                    let fixture = Self {
                        source: directory.join("source.md"),
                        directory,
                    };
                    fs::write(&fixture.source, ORIGINAL)?;
                    return Ok(fixture);
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.source);
        // Retain only an explicitly reported screenshot, not test documents.
        let _ = fs::remove_dir(&self.directory);
    }
}

struct Probe {
    gui: Gui,
    fixture: Fixture,
    window: Option<window::Id>,
    phase: Phase,
    frames: u32,
    started: Instant,
    outcome: Outcome,
    draw_barrier: DrawBarrier,
    source_fingerprint: Option<u64>,
    pending_gui: Task<Message>,
}
impl Probe {
    fn new(fixture: Fixture, outcome: Outcome) -> (Self, Task<Message>) {
        let started = Instant::now();
        let gui = Gui::open(
            crate::reference_workspace_host(),
            std::slice::from_ref(&fixture.source),
        );
        // A separate timer thread does not block Iced's executor or the UI.
        // This diagnostic deadline is not a product responsiveness budget.
        let (send, receive) = iced::futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(120));
            let _ = send.send(());
        });
        (
            Self {
                gui,
                fixture,
                window: None,
                phase: Phase::Waiting,
                frames: 0,
                started,
                outcome,
                draw_barrier: DrawBarrier::default(),
                source_fingerprint: None,
                pending_gui: Task::none(),
            },
            Task::perform(
                async {
                    let _ = receive.await;
                },
                |_| Message::Deadline,
            ),
        )
    }
    fn view(&self) -> Element<'_, Message> {
        self.gui.view().map(|_| Message::Interrupted)
    }
    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            window::open_events().map(Message::Opened),
            window::close_requests().map(|_| Message::Interrupted),
            window::frames()
                .with(self.draw_barrier.clone())
                .filter_map(|(barrier, at)| {
                    // No application message here: a message rebuilds layout and
                    // expires the weak editor/paragraph primitives just drawn by
                    // Iced 0.14. Signal the screenshot Task directly instead.
                    barrier.observe(at);
                    None::<Message>
                }),
        ])
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        if self.phase == Phase::Terminal {
            return Task::none();
        }
        match message {
            Message::Opened(id) if self.phase == Phase::Waiting => {
                self.window = Some(id);
                self.phase = Phase::Initial;
            }
            Message::Opened(_) => return self.finish(Err("unexpected additional window".into())),
            Message::Deadline => {
                return self.finish(Err("live probe exceeded its diagnostic deadline".into()));
            }
            Message::Interrupted => {
                return self.finish(Err(
                    "live probe interrupted by external input or close".into()
                ));
            }
            Message::Capture(capture) => {
                if let Err(error) = self.accept_capture(capture) {
                    return self.finish(Err(error));
                }
                if self.phase == Phase::Terminal {
                    return self.finish(Ok(Report {
                        screenshot: self.fixture.directory.join("secondary.bmp"),
                        frames: self.frames,
                        elapsed: self.started.elapsed(),
                    }));
                }
            }
            Message::ReadbackLost => {
                return self.finish(Err("own-window redraw barrier cancelled".into()));
            }
        }
        let Some(id) = self.window else {
            return self.finish(Err("redraw wait without a window".into()));
        };
        let barrier = self.draw_barrier.clone();
        // then() consumes the intermediate ID inside the Task. Only the final
        // screenshot becomes an application message, after pixels are read.
        let capture = Task::perform(
            async move {
                // Arm only after prior GUI tasks (including focus) have finished.
                let (sender, receive) = iced::futures::channel::oneshot::channel();
                *barrier.0.lock().expect("draw barrier mutex poisoned") = Some(DrawWait {
                    changed_at: Instant::now(),
                    id,
                    sender,
                });
                receive.await.ok()
            },
            |id| id,
        )
        .then(|id| match id {
            Some(id) => window::screenshot(id).map(Message::Capture),
            None => Task::done(Message::ReadbackLost),
        });
        std::mem::replace(&mut self.pending_gui, Task::none()).chain(capture)
    }
    fn send(&mut self, message: GuiMessage) -> Result<(), String> {
        // Commands run synchronously, but their UI-only operations (source
        // focus) must reach the actual widget tree before the next readback.
        // No clipboard, close or destination-confirmation workflow is scripted.
        let task = self.gui.update(message).map(|_| Message::Interrupted);
        self.pending_gui = std::mem::replace(&mut self.pending_gui, Task::none()).chain(task);
        if !self.gui.notice.is_empty() {
            return Err("GUI command refused during live probe".into());
        }
        Ok(())
    }
    fn token(&self) -> Result<crate::ViewToken, String> {
        self.gui
            .bridge
            .as_ref()
            .map(|b| b.token())
            .ok_or_else(|| "source attachment unavailable".into())
    }
    fn assert_source(&self, expected: &str, saved: bool) -> Result<(), String> {
        let bridge = self
            .gui
            .bridge
            .as_ref()
            .ok_or("source attachment unavailable")?;
        if bridge.content().text() != expected.replace("\r\n", "\n") {
            return Err("source projection disagrees with the expected native edit".into());
        }
        if saved
            && fs::read(&self.fixture.source).map_err(|_| "cannot read synthetic saved file")?
                != expected.as_bytes()
        {
            return Err("saved bytes did not preserve native contents and CRLF".into());
        }
        Ok(())
    }
    fn accept_capture(&mut self, capture: window::Screenshot) -> Result<(), String> {
        validate_capture(&capture)?;
        let bounds = self
            .gui
            .bridge
            .as_ref()
            .and_then(|b| b.source_viewport.get())
            .ok_or("source viewport was not drawn")?;
        let fingerprint = match source_fingerprint(&capture, bounds) {
            Ok(value) => value,
            Err(error) => {
                let path = self.fixture.directory.join("failed.bmp");
                if let Ok(file) = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    && write_bmp(io::BufWriter::new(file), &capture).is_ok()
                {
                    return Err(format!("{error}; own_window={}", path.display()));
                }
                return Err(error);
            }
        };
        if self.source_fingerprint == Some(fingerprint) {
            return Err("source readback did not change after a native presentation update".into());
        }
        self.source_fingerprint = Some(fingerprint);
        self.frames += 1;
        match self.phase {
            Phase::Initial => {
                self.assert_source(ORIGINAL, true)?;
                self.send(GuiMessage::QueryChanged(self.token()?, "cat".into()))?;
                self.send(GuiMessage::Find(self.token()?))?;
                self.send(GuiMessage::SelectMatches(self.token()?))?;
                self.phase = Phase::Selected;
            }
            Phase::Selected => {
                let bridge = self
                    .gui
                    .bridge
                    .as_mut()
                    .ok_or("source attachment unavailable")?;
                let snapshot = bridge
                    .native_selection(&bridge.token())
                    .map_err(|_| "cannot inspect native selections")?
                    .ok_or("native selections unavailable")?;
                if snapshot.selections.selections().len() != 2
                    || bridge.secondary_geometry_usage().emitted_regions == 0
                {
                    return Err(
                        "live secondary selection was not rendered from two native targets".into(),
                    );
                }
                let path = self.fixture.directory.join("secondary.bmp");
                let file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .map_err(|_| "cannot create own-window screenshot")?;
                write_bmp(io::BufWriter::new(file), &capture)
                    .map_err(|_| "cannot encode own-window screenshot")?;
                self.send(GuiMessage::Widget(
                    self.token()?,
                    text_editor::Action::Edit(text_editor::Edit::Insert('X')),
                ))?;
                self.send(GuiMessage::Shortcut(
                    self.token()?,
                    super::Shortcut::Command(Command::Save),
                ))?;
                self.assert_source(EDITED, true)?;
                self.phase = Phase::Edited;
            }
            Phase::Edited => {
                self.assert_source(EDITED, true)?;
                self.send(GuiMessage::Shortcut(
                    self.token()?,
                    super::Shortcut::Command(Command::Undo),
                ))?;
                self.send(GuiMessage::Shortcut(
                    self.token()?,
                    super::Shortcut::Command(Command::Save),
                ))?;
                self.assert_source(ORIGINAL, true)?;
                self.phase = Phase::Restored;
            }
            Phase::Restored => {
                self.assert_source(ORIGINAL, true)?;
                self.phase = Phase::Terminal;
            }
            _ => return Err("capture outside the closed live-probe phase vocabulary".into()),
        }
        Ok(())
    }
    fn finish(&mut self, mut result: Result<Report, String>) -> Task<Message> {
        // Only synthetic documents are owned here. Cancellation may discard
        // them, but unresolved engine obligations still refuse close normally.
        if let Some(bridge) = self.gui.bridge.as_mut()
            && bridge.close(result.is_err()).is_err()
        {
            result = Err("native close refused during live probe".into())
        }
        self.phase = Phase::Terminal;
        self.draw_barrier
            .0
            .lock()
            .expect("draw barrier mutex poisoned")
            .take();
        *self
            .outcome
            .lock()
            .expect("qualification outcome mutex poisoned") = Some(result);
        iced::exit()
    }
}

fn validate_capture(capture: &window::Screenshot) -> Result<(), String> {
    let pixels = (capture.size.width as usize)
        .checked_mul(capture.size.height as usize)
        .filter(|n| *n > 0 && *n <= MAX_PIXELS)
        .ok_or("invalid or excessive screenshot dimensions")?;
    if pixels.checked_mul(4) != Some(capture.rgba.len())
        || !capture.scale_factor.is_finite()
        || capture.scale_factor <= 0.0
    {
        return Err("invalid screenshot framing or scale".into());
    }
    let first = &capture.rgba[..4];
    if !capture.rgba.chunks_exact(4).any(|p| p != first) {
        return Err("own-window screenshot is uniform".into());
    }
    Ok(())
}

fn source_fingerprint(
    capture: &window::Screenshot,
    bounds: iced::Rectangle,
) -> Result<u64, String> {
    validate_capture(capture)?;
    let scale = capture.scale_factor;
    let x0 = (bounds.x * scale).ceil().max(0.0) as usize;
    let y0 = (bounds.y * scale).ceil().max(0.0) as usize;
    let x1 = ((bounds.x + bounds.width) * scale).floor().max(0.0) as usize;
    let y1 = ((bounds.y + bounds.height) * scale).floor().max(0.0) as usize;
    let width = capture.size.width as usize;
    let height = capture.size.height as usize;
    if x0 >= x1 || y0 >= y1 || x1 > width || y1 > height {
        return Err("drawn source viewport is outside the captured window".into());
    }
    let first = &capture.rgba[(y0 * width + x0) * 4..(y0 * width + x0 + 1) * 4];
    let mut different = false;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for y in y0..y1 {
        let row = &capture.rgba[(y * width + x0) * 4..(y * width + x1) * 4];
        different |= row.chunks_exact(4).any(|p| p != first);
        row.hash(&mut hash);
    }
    if !different {
        return Err("source viewport readback is blank/uniform".into());
    }
    Ok(hash.finish())
}

// Uncompressed top-down 32-bit BMP is used only as a diagnostic artifact. No
// image parser/encoder dependency enters normal UI or engine builds.
fn write_bmp(mut output: impl Write, capture: &window::Screenshot) -> io::Result<()> {
    validate_capture(capture).map_err(io::Error::other)?;
    let mut header = [0u8; 54];
    header[..2].copy_from_slice(b"BM");
    header[2..6].copy_from_slice(&(54 + capture.rgba.len() as u32).to_le_bytes());
    header[10..14].copy_from_slice(&54u32.to_le_bytes());
    header[14..18].copy_from_slice(&40u32.to_le_bytes());
    header[18..22].copy_from_slice(&capture.size.width.to_le_bytes());
    header[22..26].copy_from_slice(&(-(capture.size.height as i32)).to_le_bytes());
    header[26..28].copy_from_slice(&1u16.to_le_bytes());
    header[28..30].copy_from_slice(&32u16.to_le_bytes());
    header[34..38].copy_from_slice(&(capture.rgba.len() as u32).to_le_bytes());
    output.write_all(&header)?;
    for rgba in capture.rgba.chunks_exact(4) {
        output.write_all(&[rgba[2], rgba[1], rgba[0], rgba[3]])?
    }
    output.flush()
}

/// Explicitly opens one real window containing only freshly created synthetic
/// documents. Captures that window through Iced, never the desktop or other apps.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let outcome = Arc::new(Mutex::new(None));
    let shared = Arc::clone(&outcome);
    // Iced requires a reusable initializer; take the one fixture exactly once.
    let fixture = Mutex::new(Some(fixture));
    iced::application(
        move || {
            Probe::new(
                fixture
                    .lock()
                    .expect("fixture mutex poisoned")
                    .take()
                    .expect("qualification initializer called twice"),
                Arc::clone(&shared),
            )
        },
        Probe::update,
        Probe::view,
    )
    .title("Vellum — synthetic live qualification")
    .subscription(Probe::subscription)
    .exit_on_close_request(false)
    .window_size((960.0, 720.0))
    .run()?;
    match outcome
        .lock()
        .expect("qualification outcome mutex poisoned")
        .take()
    {
        Some(Ok(report)) => {
            println!(
                "Live own-window probe passed: captures={}, bootstrap_to_final_capture_ms={}, screenshot={}",
                report.frames,
                report.elapsed.as_millis(),
                report.screenshot.display()
            );
            println!(
                "Verified native select → multi-edit → CRLF save → atomic undo → CRLF save → close. No system input injection or clipboard workflow; not physical-input/IME/accessibility/bidi qualification."
            );
            Ok(())
        }
        Some(Err(error)) => Err(error.into()),
        None => Err("event loop exited without a qualification result".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn capture(rgba: Vec<u8>, width: u32, height: u32) -> window::Screenshot {
        window::Screenshot::new(rgba, iced::Size::new(width, height), 1.0)
    }
    #[test]
    fn screenshot_framing_and_uniform_readbacks_refuse() {
        for image in [
            capture(vec![], 0, 1),
            capture(vec![0; 4], 2, 1),
            capture(vec![0; 8], 2, 1),
            capture(vec![], u32::MAX, u32::MAX),
        ] {
            assert!(validate_capture(&image).is_err())
        }
    }
    #[test]
    fn screenshot_artifact_preserves_top_down_pixel_order_without_a_runtime_image_dependency() {
        let image = capture(vec![1, 2, 3, 255, 4, 5, 6, 255], 1, 2);
        let mut bmp = Vec::new();
        write_bmp(&mut bmp, &image).unwrap();
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(
            u32::from_le_bytes(bmp[2..6].try_into().unwrap()) as usize,
            bmp.len()
        );
        assert_eq!(i32::from_le_bytes(bmp[22..26].try_into().unwrap()), -2);
        assert_eq!(&bmp[54..], &[3, 2, 1, 255, 6, 5, 4, 255]);
    }

    #[test]
    fn nonblank_window_does_not_certify_a_blank_source_region() {
        let image = capture(
            vec![0, 0, 0, 255, 0, 0, 0, 255, 1, 2, 3, 255, 0, 0, 0, 255],
            2,
            2,
        );
        assert!(validate_capture(&image).is_ok());
        assert!(
            source_fingerprint(
                &image,
                iced::Rectangle::new(iced::Point::ORIGIN, iced::Size::new(2.0, 1.0))
            )
            .is_err()
        );
        assert!(
            source_fingerprint(
                &image,
                iced::Rectangle::new(iced::Point::ORIGIN, iced::Size::new(2.0, 2.0))
            )
            .is_ok()
        );
        assert!(
            source_fingerprint(
                &image,
                iced::Rectangle::new(iced::Point::ORIGIN, iced::Size::new(3.0, 2.0))
            )
            .is_err()
        );
    }

    #[test]
    fn old_redraws_cannot_trigger_readback_and_each_barrier_is_one_shot() {
        let barrier = DrawBarrier::default();
        let old = Instant::now();
        let (sender, mut receive) = iced::futures::channel::oneshot::channel();
        let id = window::Id::unique();
        *barrier.0.lock().unwrap() = Some(DrawWait {
            changed_at: Instant::now(),
            id,
            sender,
        });
        barrier.observe(old);
        assert_eq!(receive.try_recv().unwrap(), None);
        barrier.observe(Instant::now());
        assert_eq!(receive.try_recv().unwrap(), Some(id));
        barrier.observe(Instant::now());
        assert!(barrier.0.lock().unwrap().is_none());
    }

    #[test]
    fn interruption_is_terminal_and_does_not_write_the_synthetic_document() {
        let fixture = Fixture::new().unwrap();
        let outcome = Arc::new(Mutex::new(None));
        let (mut probe, _) = Probe::new(fixture, Arc::clone(&outcome));
        let _ = probe.update(Message::Opened(window::Id::unique()));
        let _ = probe.update(Message::Interrupted);
        assert_eq!(probe.phase, Phase::Terminal);
        assert!(matches!(&*outcome.lock().unwrap(), Some(Err(_))));
        let _ = probe.update(Message::Opened(window::Id::unique()));
        assert_eq!(probe.phase, Phase::Terminal);
        assert_eq!(probe.frames, 0);
        assert_eq!(
            fs::read(&probe.fixture.source).unwrap(),
            ORIGINAL.as_bytes()
        );
    }
}

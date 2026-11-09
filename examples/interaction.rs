//! Headless widget/controller service workload. No window/event-loop, drawing,
//! filesystem-write or process cold-start latency is measured. Attachment and
//! explicit close service are separate columns; no product SLO is selected.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Workload {
    ShortLines,
    LongLine,
    UnicodeLines,
}
#[cfg(target_os = "macos")]
impl Workload {
    fn label(self) -> &'static str {
        match self {
            Self::ShortLines => "short-lines",
            Self::LongLine => "long-line",
            Self::UnicodeLines => "unicode-lines",
        }
    }
    fn unit(self) -> &'static str {
        match self {
            Self::ShortLines => "prose\tcode\r\n",
            Self::LongLine => "prose code ",
            Self::UnicodeLines => "Grüße 界 👩‍💻 code\r\n",
        }
    }
}
#[cfg(target_os = "macos")]
struct Options {
    sizes: Vec<usize>,
    samples: usize,
    warmup: usize,
    workload: Workload,
    native_unicode: bool,
}
#[cfg(target_os = "macos")]
impl Options {
    fn parse(args: &[String]) -> Result<Self, &'static str> {
        let mut options = Self {
            sizes: vec![1024, 64000],
            samples: 32,
            warmup: 8,
            workload: Workload::ShortLines,
            native_unicode: false,
        };
        if args.len() > 10 || !args.len().is_multiple_of(2) {
            return Err("expected --source-bytes N --samples N --warmup N --workload NAME");
        }
        let mut seen = [false; 5];
        for pair in args.chunks_exact(2) {
            if pair[0] == "--unicode-profile" {
                if seen[4] {
                    return Err("duplicate option");
                }
                options.native_unicode = match pair[1].as_str() {
                    "native" => true,
                    "legacy" => false,
                    _ => return Err("unknown Unicode profile"),
                };
                seen[4] = true;
                continue;
            }
            if pair[0] == "--workload" {
                if seen[3] {
                    return Err("duplicate option");
                }
                options.workload = match pair[1].as_str() {
                    "short-lines" => Workload::ShortLines,
                    "long-line" => Workload::LongLine,
                    "unicode-lines" => Workload::UnicodeLines,
                    _ => return Err("unknown workload"),
                };
                seen[3] = true;
                continue;
            }
            let value: usize = pair[1]
                .parse()
                .map_err(|_| "expected bounded unsigned integer")?;
            let index = match pair[0].as_str() {
                "--source-bytes" if (12..=8 * 1024 * 1024).contains(&value) => {
                    options.sizes = vec![value];
                    0
                }
                "--samples" if (32..=1024).contains(&value) => {
                    options.samples = value;
                    1
                }
                "--warmup" if value <= 256 => {
                    options.warmup = value;
                    2
                }
                _ => return Err("unknown option or experimental limit exceeded"),
            };
            if seen[index] {
                return Err("duplicate option");
            }
            seen[index] = true;
        }
        if options
            .sizes
            .iter()
            .any(|size| *size < options.workload.unit().len())
        {
            return Err("source size smaller than workload unit");
        }
        if options.native_unicode && options.sizes.iter().any(|n| *n >= 65536) {
            return Err(
                "native reference workload requires source bytes below 65536 to leave insertion headroom",
            );
        }
        Ok(options)
    }
}
#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use iced::widget::text_editor::{Action, Edit};
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::Instant,
    };
    use vellum_editor::reference_profile;
    use vellum_engine::instrumentation::NativeTimingProfileV1;
    use vellum_iced::Bridge;
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--help"] {
        println!(
            "iced-interaction [--source-bytes N] [--samples N] [--warmup N] [--workload NAME]\nExperimental headless controller workload, not a shipped capacity or latency SLO.\nBounds: 12..=8388608 source bytes; 32..=1024 samples; 0..=256 warmup iterations.\nDefault: 1024/64000-byte cohorts, 32 samples, 8 warmup; capture off/enabled/saturated.\nTwo edits plus projection oracles per sample; separate attachment/close clocks.\nNo drawing, process cold start or save latency."
        );
        println!(
            "--workload short-lines|long-line|unicode-lines selects a fixed synthetic shape (default: short-lines)."
        );
        println!(
            "--unicode-profile legacy|native (default: legacy). Native uses the qualified 64 KiB profile and includes preview + evidence generation; no OS clipboard access."
        );
        return Ok(());
    }
    let options = Options::parse(&args)?;
    if options.native_unicode {
        eprintln!(
            "profile=native-unicode18; headless widget/provider/engine service, not drawing or cold startup"
        );
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(self.0.join("source"));
            let _ = fs::remove_dir(&self.0);
        }
    }
    println!(
        "source_bytes,capture,samples,p50_ns,p95_ns,p99_ns,max_ns,capture_drops_before_close,trace_window_overwrites_before_close,insert_p99_ns,backspace_p99_ns,readback_p99_ns,workload,attach_ns,close_ns"
    );
    for bytes in options.sizes {
        for capacity in [0, 256, 1] {
            let directory = loop {
                let path = std::env::temp_dir().join(format!(
                    "vellum-iced-perf-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&path) {
                    Ok(()) => break Fixture(path),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => return Err(e.into()),
                }
            };
            let path = directory.0.join("source");
            let unit = options.workload.unit();
            let original = unit.repeat(bytes / unit.len());
            let visual = original.replace("\r\n", "\n");
            fs::write(&path, &original)?;
            let mut config = reference_profile();
            // Experiment-only envelope; the reference apps keep their explicit
            // 64-KiB limits. Fund complete candidates/baselines, not only input.
            let limit = bytes + 16;
            config.engine.limits.source.max_source_bytes =
                config.engine.limits.source.max_source_bytes.max(limit);
            config.engine.limits.max_staged_bytes =
                config.engine.limits.max_staged_bytes.max(limit * 4);
            config.filesystem.max_bytes = config.filesystem.max_bytes.max(limit);
            let resources = config
                .engine
                .resources
                .as_mut()
                .ok_or("missing resource profile")?;
            resources.max_blob_bytes = resources.max_blob_bytes.max(limit);
            resources.max_held_artifact_bytes = resources.max_held_artifact_bytes.max(limit * 4);
            resources.max_future_baseline_bytes =
                resources.max_future_baseline_bytes.max(limit * 4);
            resources.max_held_read_bytes = resources.max_held_read_bytes.max(limit * 4);
            config.capture_capacity = capacity;
            config.timing = if capacity == 0 {
                NativeTimingProfileV1::Off
            } else {
                NativeTimingProfileV1::ProcessMonotonicV1
            };
            let nanos = |start: Instant, end: Instant| {
                end.checked_duration_since(start)
                    .map(|d| d.as_nanos())
                    .ok_or("monotonic timing unavailable")
            };
            let attaching = Instant::now();
            let mut bridge = Bridge::open(std::slice::from_ref(&path), config)?;
            if options.native_unicode {
                let mut profile = vellum_editor::NativeUnicodeProfile::reference();
                profile.capture_capacity = capacity;
                profile.timing = if capacity == 0 {
                    vellum_editor::NativeUnicodeTiming::Off
                } else {
                    vellum_editor::NativeUnicodeTiming::ProcessService
                };
                bridge.enable_native_unicode(&bridge.token(), profile)?;
            }
            let attach_ns = nanos(attaching, Instant::now())?;
            let token = bridge.token();
            let mut samples = Vec::with_capacity(options.samples);
            let mut inserts = Vec::with_capacity(options.samples);
            let mut backspaces = Vec::with_capacity(options.samples);
            let mut readbacks = Vec::with_capacity(options.samples);
            for index in 0..options.samples + options.warmup {
                let started = Instant::now();
                bridge.widget(&token, Action::Edit(Edit::Insert('x')))?;
                let inserted = Instant::now();
                if !bridge.dirty() || !bridge.content().text().starts_with('x') {
                    return Err("typed projection oracle failed".into());
                }
                let deleting = Instant::now();
                bridge.widget(&token, Action::Edit(Edit::Backspace))?;
                let deleted = Instant::now();
                let restored = bridge.content().text();
                let finished = Instant::now();
                let elapsed = nanos(started, finished)?;
                if bridge.dirty() || restored != visual {
                    return Err("restoration oracle failed".into());
                }
                if index >= options.warmup {
                    samples.push(elapsed);
                    inserts.push(nanos(started, inserted)?);
                    backspaces.push(nanos(deleting, deleted)?);
                    readbacks.push(
                        nanos(inserted, deleting)?
                            .checked_add(nanos(deleted, finished)?)
                            .ok_or("timing total overflow")?,
                    );
                }
            }
            samples.sort_unstable();
            inserts.sort_unstable();
            backspaces.sort_unstable();
            readbacks.sort_unstable();
            let percentile =
                |values: &[u128], p: usize| values[(values.len() * p).div_ceil(100) - 1];
            let drops = bridge.dropped_diagnostics();
            let overwritten = bridge
                .diagnostic_window_overwrites()
                .map_or_else(|| "unknown".into(), |n| n.to_string());
            if bridge.full_projection_refreshes() != Some(1) {
                return Err("ordinary typing rebuilt the full projection".into());
            }
            if (capacity == 1) != (drops > 0) {
                return Err("unexpected diagnostic loss population".into());
            }
            let closing = Instant::now();
            bridge.close(false)?;
            let close_ns = nanos(closing, Instant::now())?;
            if !bridge.is_closed() || fs::read_to_string(&path)? != original {
                return Err("cleanup/disk oracle failed".into());
            }
            if options.native_unicode
                && (bridge.native_boundary_usage().retained_bytes != 0
                    || bridge.native_unicode_usage().is_none_or(|u| {
                        u.refused != 0
                            || u.completed != (4 * (options.samples + options.warmup)) as u64
                    }))
            {
                return Err("native preview/evidence population or cleanup oracle failed".into());
            }
            println!(
                "{},{capacity},{},{},{},{},{},{drops},{overwritten},{},{},{},{},{attach_ns},{close_ns}",
                original.len(),
                options.samples,
                percentile(&samples, 50),
                percentile(&samples, 95),
                percentile(&samples, 99),
                samples[samples.len() - 1],
                percentile(&inserts, 99),
                percentile(&backspaces, 99),
                percentile(&readbacks, 99),
                options.workload.label()
            );
        }
    }
    Ok(())
}
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("iced interaction workload currently requires macOS");
    std::process::exit(1);
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::Options;
    #[test]
    fn experiment_options_are_explicit_bounded_and_do_not_change_app_defaults() {
        let defaults = Options::parse(&[]).unwrap();
        assert_eq!(defaults.sizes, [1024, 64000]);
        let parse =
            |args: &[&str]| Options::parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let larger = parse(&[
            "--source-bytes",
            "1048576",
            "--samples",
            "512",
            "--warmup",
            "32",
        ])
        .unwrap();
        assert_eq!(
            (larger.sizes[0], larger.samples, larger.warmup),
            (1048576, 512, 32)
        );
        assert!(parse(&["--source-bytes", "8388609"]).is_err());
        assert!(parse(&["--samples", "0"]).is_err());
        assert!(parse(&["--warmup", "1", "--warmup", "2"]).is_err());
        assert!(parse(&["--samples"]).is_err());
        assert!(parse(&["--unknown", "1"]).is_err());
        assert_eq!(
            parse(&["--workload", "long-line"]).unwrap().workload,
            super::Workload::LongLine
        );
        assert!(parse(&["--workload", "unknown"]).is_err());
        assert!(parse(&["--source-bytes", "12", "--workload", "unicode-lines"]).is_err());
        assert!(!defaults.native_unicode);
        assert!(
            parse(&["--unicode-profile", "native"])
                .unwrap()
                .native_unicode
        );
        assert!(parse(&["--unicode-profile", "unknown"]).is_err());
        assert!(parse(&["--unicode-profile", "native", "--unicode-profile", "legacy"]).is_err());
        assert!(parse(&["--unicode-profile", "native", "--source-bytes", "65536"]).is_err());
    }
}

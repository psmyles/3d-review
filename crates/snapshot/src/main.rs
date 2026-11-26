//! `review-snapshot`: render models, their `_LOD<n>` levels or an Opt stack's
//! output to PNG without a window, through the viewer's own renderer.
//!
//! A developer tool. It exists so a change to the optimizer (or the renderer)
//! can be looked at — and measured — from a script while the machine is being
//! used for something else: nothing here opens a window, takes focus or reads
//! input. The GPU is `review-render`'s `Gpu::headless`, which draws every frame
//! exactly as the viewer does into a texture it then reads back.
//!
//! Each look writes one contact sheet (a row per subject, a column per view).
//! `--clip` poses every subject on one frame of a clip instead of its bind pose,
//! and `--curves` / `--skeleton` / `--select` draw what the viewer draws over an
//! animation - see [`animation`].
//! `--compare` additionally renders every subject's interpolated vertex normals
//! and reports how far each one's are from the first subject's, pixel by pixel
//! from the same cameras — see [`compare`].

#![forbid(unsafe_code)]

mod animation;
mod capture;
mod cli;
mod compare;
mod inputs;
mod inspect;
mod sheet;

use std::process::ExitCode;

use anyhow::{Result, bail};
use review_model::Bounds;

use crate::capture::Capture;
use crate::cli::Look;
use crate::compare::NormalError;
use crate::sheet::Row;

/// Warnings and errors, to standard error. Without a logger the `log` facade drops
/// everything, and with it the only account of why a frame came out wrong: sokol's
/// validation failures (`rhi/sokol_log.rs`) and, in a debug build on Linux, the
/// Vulkan validation layer's.
struct StderrLog;

impl log::Log for StderrLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("{}: {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

static STDERR_LOG: StderrLog = StderrLog;

fn main() -> ExitCode {
    if log::set_logger(&STDERR_LOG).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let Some(args) = cli::parse(std::env::args().skip(1))? else {
        print!("{}", cli::USAGE);
        return Ok(());
    };

    let mut subjects = Vec::new();
    for spec in &args.inputs {
        subjects.extend(inputs::load(spec)?);
    }
    if !args.colliders.is_empty() {
        let names = review_audit::glob::GlobSet::new(&args.colliders)
            .map_err(|_| anyhow::anyhow!("--colliders: a pattern does not parse"))?;
        for subject in &mut subjects {
            subject.wire = (0..subject.model.nodes.len() as u32)
                .filter(|node| {
                    names.matches(&subject.model.nodes[*node as usize].name)
                        && subject.hidden.binary_search(node).is_err()
                })
                .collect();
        }
    }
    for (index, subject) in subjects.iter().enumerate() {
        let (triangles, vertices) = subject.counts();
        println!(
            "[{index}] {}: {triangles} triangles, {vertices} GPU vertices",
            subject.label
        );
    }

    if args.inspect && subjects.len() > 1 {
        println!("vertices against [0] {}:", subjects[0].label);
        for (index, subject) in subjects.iter().enumerate().skip(1) {
            println!("  [{index}] {}", subject.label);
            inspect::report(&subjects[0], subject);
        }
    }

    if !args.render {
        return Ok(());
    }

    let attachments = animation::load_attachments(&args.attach)?;
    let posed: Vec<Option<animation::Posed>> = subjects
        .iter()
        .map(|subject| {
            let attached = animation::attach(&subject.model, &attachments);
            args.clip
                .map(|clip| {
                    animation::pose(
                        &subject.model,
                        &attached,
                        clip,
                        args.curves,
                        &args.select,
                        &subject.hidden,
                    )
                })
                .transpose()
        })
        .collect::<Result<_>>()?;
    // A posed subject frames on its clip's whole motion, as the viewer does.
    let bounds = |index: usize| {
        posed[index]
            .as_ref()
            .and_then(|posed| posed.envelope)
            .or_else(|| subjects[index].bounds())
    };
    let shared = match args.frame_on {
        cli::FrameOn::Subject(index) if index < subjects.len() => bounds(index),
        cli::FrameOn::Subject(index) => bail!(
            "--frame {index}: there are only {} subjects",
            subjects.len()
        ),
        cli::FrameOn::All | cli::FrameOn::Each => {
            (0..subjects.len()).filter_map(bounds).reduce(union)
        }
    };
    let Some(shared) = shared.filter(|bounds| !bounds.is_empty()) else {
        bail!("nothing to frame: every subject is empty");
    };
    if args.frame_on == cli::FrameOn::Each {
        println!("framing each subject on its own bounds");
    } else {
        println!(
            "framing bounds: min {:.3?} max {:.3?} (focus x,y,z in a view is a fraction of these)",
            shared.min.to_array(),
            shared.max.to_array()
        );
    }
    // The bounds each subject's cells are framed on.
    let framing: Vec<Bounds> = (0..subjects.len())
        .map(|index| match args.frame_on {
            cli::FrameOn::Each => bounds(index)
                .filter(|own| !own.is_empty())
                .unwrap_or(shared),
            _ => shared,
        })
        .collect();

    std::fs::create_dir_all(&args.out)?;
    let mut capture = Capture::new(
        args.size,
        args.chrome,
        args.background,
        args.transparent,
        args.grid,
        args.skeleton,
        args.curve_style,
    )?;
    capture.uv_density = args.uv_density;
    capture.index = args.index;
    if let Some(square) = args.checker_square {
        capture.checker_tiling = 1.0 / (review_render::CHECKER_SQUARES * square * args.uv_density);
    }
    let columns: Vec<String> = args.views.iter().map(|view| view.name.clone()).collect();
    let cameras_for = |capture: &Capture, index: usize| -> Vec<_> {
        args.views
            .iter()
            .map(|view| capture.camera(view, framing[index]))
            .collect()
    };
    let cameras = cameras_for(&capture, 0);
    let mut written: Vec<std::path::PathBuf> = Vec::new();

    for &look in &args.looks {
        let mut rows = Vec::with_capacity(subjects.len());
        for (index, subject) in subjects.iter().enumerate() {
            let cameras = cameras_for(&capture, index);
            let mut cells = Vec::with_capacity(cameras.len());
            for (view, &camera) in cameras.iter().enumerate() {
                let image = capture.render(subject, posed[index].as_ref(), camera, look)?;
                if args.singles {
                    let single = args.out.join(format!(
                        "{}_{}_s{index}_v{view}.png",
                        args.name,
                        look.name()
                    ));
                    image.save(&single)?;
                    written.push(single);
                }
                cells.push(image);
            }
            // The row says how big the part is: framed each on its own, the
            // pictures no longer do.
            let size = subject
                .bounds()
                .map(|bounds| {
                    let size = bounds.size();
                    format!("  {:.3} x {:.3} x {:.3} m", size.x, size.y, size.z)
                })
                .unwrap_or_default();
            rows.push(Row {
                label: format!("[{index}] {}{size}", subject.label),
                cells,
            });
        }
        let path = args.out.join(format!("{}_{}.png", args.name, look.name()));
        sheet::compose(&columns, &rows).save(&path)?;
        println!("wrote {}", path.display());
        written.push(path);
    }

    if args.compare && subjects.len() > 1 {
        let reference: Vec<_> = cameras
            .iter()
            .map(|&camera| capture.render(&subjects[0], posed[0].as_ref(), camera, Look::Normals))
            .collect::<Result<_>>()?;
        let mut rows = Vec::with_capacity(subjects.len() - 1);
        println!("shading normals against [0] {}:", subjects[0].label);
        for (index, subject) in subjects.iter().enumerate().skip(1) {
            let mut error = NormalError::default();
            let mut cells = Vec::with_capacity(cameras.len());
            for (view, &camera) in cameras.iter().enumerate() {
                let candidate =
                    capture.render(subject, posed[index].as_ref(), camera, Look::Normals)?;
                cells.push(error.add_view(&reference[view], &candidate));
            }
            println!("  [{index}] {}", error.summary());
            rows.push(Row {
                label: format!(
                    "[{index}] {}  mean {:.2} deg  silhouette {:.2}%",
                    subject.label,
                    error.mean(),
                    error.silhouette_mismatch_percent()
                ),
                cells,
            });
        }
        let path = args.out.join(format!("{}_compare.png", args.name));
        sheet::compose(&columns, &rows).save(&path)?;
        println!("wrote {}", path.display());
        written.push(path);
    }

    if let Some(report) = &args.report {
        write_report(report, &args, &subjects, &written)?;
        println!("wrote {}", report.display());
    }
    Ok(())
}

/// The run as JSON: each subject's file, label, counts and bounds in metres
/// (in the file's own axes), and every image written.
fn write_report(
    path: &std::path::Path,
    args: &cli::Args,
    subjects: &[inputs::Subject],
    written: &[std::path::PathBuf],
) -> Result<()> {
    let triple = |v: glam::Vec3| serde_json::json!([v.x, v.y, v.z]);
    let entries: Vec<serde_json::Value> = subjects
        .iter()
        .enumerate()
        .map(|(index, subject)| {
            let (triangles, gpu_vertices) = subject.counts();
            serde_json::json!({
                "index": index,
                "label": subject.label,
                "triangles": triangles,
                "gpu_vertices": gpu_vertices,
                "bounds_m": subject.bounds().filter(|bounds| !bounds.is_empty()).map(|bounds| serde_json::json!({
                    "min": triple(bounds.min),
                    "max": triple(bounds.max),
                    "size": triple(bounds.size()),
                })),
            })
        })
        .collect();
    let report = serde_json::json!({
        "schema": "3d-review.render-report",
        "version": 1,
        "framing": match args.frame_on {
            cli::FrameOn::All => "all".to_owned(),
            cli::FrameOn::Each => "each".to_owned(),
            cli::FrameOn::Subject(index) => format!("subject {index}"),
        },
        "subjects": entries,
        "outputs": written.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
    });
    if let Some(folder) = path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    {
        std::fs::create_dir_all(folder)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(&report)?)?;
    Ok(())
}

fn union(a: Bounds, b: Bounds) -> Bounds {
    Bounds {
        min: a.min.min(b.min),
        max: a.max.max(b.max),
    }
}

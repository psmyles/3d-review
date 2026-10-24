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

fn main() -> ExitCode {
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
    let framing = match args.frame_on {
        Some(index) if index < subjects.len() => bounds(index),
        Some(index) => bail!(
            "--frame {index}: there are only {} subjects",
            subjects.len()
        ),
        None => (0..subjects.len()).filter_map(bounds).reduce(union),
    };
    let Some(framing) = framing.filter(|bounds| !bounds.is_empty()) else {
        bail!("nothing to frame: every subject is empty");
    };
    println!(
        "framing bounds: min {:.3?} max {:.3?} (focus x,y,z in a view is a fraction of these)",
        framing.min.to_array(),
        framing.max.to_array()
    );

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
    let columns: Vec<String> = args.views.iter().map(|view| view.name.clone()).collect();
    let cameras: Vec<_> = args
        .views
        .iter()
        .map(|view| capture.camera(view, framing))
        .collect();

    for &look in &args.looks {
        let mut rows = Vec::with_capacity(subjects.len());
        for (index, subject) in subjects.iter().enumerate() {
            let mut cells = Vec::with_capacity(cameras.len());
            for (view, &camera) in cameras.iter().enumerate() {
                let image = capture.render(subject, posed[index].as_ref(), camera, look)?;
                if args.singles {
                    image.save(args.out.join(format!(
                        "{}_{}_s{index}_v{view}.png",
                        args.name,
                        look.name()
                    )))?;
                }
                cells.push(image);
            }
            rows.push(Row {
                label: format!("[{index}] {}", subject.label),
                cells,
            });
        }
        let path = args.out.join(format!("{}_{}.png", args.name, look.name()));
        sheet::compose(&columns, &rows).save(&path)?;
        println!("wrote {}", path.display());
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
    }
    Ok(())
}

fn union(a: Bounds, b: Bounds) -> Bounds {
    Bounds {
        min: a.min.min(b.min),
        max: a.max.max(b.max),
    }
}

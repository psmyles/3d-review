//! The command line: what to render, from where, and how it should look.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use review_render::ViewportBackground;

pub const USAGE: &str = "\
review-snapshot - headless renders through the viewer's own renderer

USAGE: review-snapshot [OPTIONS] <INPUT>...

INPUT   <file.fbx>[@<stack>][#<level>]
        @<preset.json>      run an Opt preset (as saved from the Opt workspace)
        @lods[:r1,r2,...][/err]
                            Generate LODs at these triangle ratios with the default
                            settings (0.5,0.25,0.125 when no ratios are given), and
                            optionally this error limit on every level
        #<n> or #LOD<n>     one level: the stack's level n, or the file's _LOD<n> nodes
        #all                every level as its own subject (the default with a stack)

OPTIONS
  --out <dir>           where images go (default target/snapshots)
  --name <prefix>       file-name prefix (default snap)
  --size <WxH>          size of one view (default 800x600)
  --views <list>        ';'-separated views: iso, iso-back, front, back, left, right,
                        top, bottom, or yaw,pitch[,zoom[,x,y,z]] in degrees with x,y,z
                        a focus point in 0..1 of the framing bounds (default iso;iso-back)
  --look <list>         ','-separated: shaded, matte, normals, wire, shaded-wire,
                        checker
                        (default shaded)
  --frame <n>           frame every view on subject n's bounds instead of on all of them
  --chrome <l,t,r,b>    pixels of each view the viewer's chrome covers on the left, top,
                        right and bottom (its panels and bars): frame into what they
                        leave, as the viewer does, and dim them so the sheet shows it
  --background <name>   black, grey25, grey50, grey75, white, gradient (default grey25)
  --grid                draw the ground grid
  --compare             measure each subject's shading normals against the first's,
                        and write a heat-map sheet of where they disagree
  --singles             also write each view as its own image
  --inspect             report how each subject's vertices and normals relate to
                        the first subject's (kept or moved, copied or recomputed)
  --no-render           skip the images (with --inspect)
  --clip <n>[:<frame>]  pose every subject on its clip n (0-based) at that frame
                        (default the first), framing on the clip's whole motion
  --attach <file.fbx>   attach an animation file to every subject, as the viewer's
                        File > Add Animations does (repeatable); its clips number on
                        from the subject's own in --clip
  --curves              with --clip: draw the clip's animation curves
  --skeleton            draw the skeleton overlay
  --select <names>      ','-separated node names, selected as an Outliner click
                        would: bones narrow the curves to their own, a mesh or group
                        to everything under it
";

/// Everything the command line asked for.
pub struct Args {
    pub inputs: Vec<InputSpec>,
    pub out: PathBuf,
    pub name: String,
    pub size: (u32, u32),
    pub views: Vec<View>,
    pub looks: Vec<Look>,
    pub frame_on: Option<usize>,
    /// Pixels of each view covered on the left, top, right and bottom.
    pub chrome: [u32; 4],
    pub background: ViewportBackground,
    pub grid: bool,
    pub compare: bool,
    pub singles: bool,
    pub inspect: bool,
    pub render: bool,
    pub clip: Option<ClipSpec>,
    /// Animation files attached to every subject, in order.
    pub attach: Vec<PathBuf>,
    pub curves: bool,
    pub skeleton: bool,
    pub select: Vec<String>,
}

/// Which clip to pose the subjects on, and on which of its frames.
#[derive(Clone, Copy)]
pub struct ClipSpec {
    pub index: usize,
    /// 0-based; the first frame when `None`, clamped to the last.
    pub frame: Option<usize>,
}

/// One `INPUT` argument, parsed.
pub struct InputSpec {
    pub path: PathBuf,
    pub stack: Option<StackSpec>,
    pub level: LevelSpec,
}

/// What to run over the file before rendering it.
pub enum StackSpec {
    Preset(PathBuf),
    /// Triangle ratios, and an error limit for every level (the default's when `None`).
    Lods(Vec<f32>, Option<f32>),
}

/// Which part of the file (or of the stack's output) a subject is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LevelSpec {
    /// The whole file, or every level when a stack runs.
    Default,
    /// Every level as its own subject.
    All,
    One(u32),
}

/// Where the camera looks from.
#[derive(Clone)]
pub struct View {
    pub name: String,
    pub yaw_degrees: f32,
    pub pitch_degrees: f32,
    pub zoom: f32,
    /// A point to centre on, in `0..1` of the framing bounds; `None` keeps the
    /// framed centre.
    pub focus: Option<[f32; 3]>,
}

/// How a subject is shaded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Look {
    /// The viewer's defaults: source materials, IBL, ambient occlusion, tone map.
    Shaded,
    /// The same lighting over one uniform matte grey material.
    Matte,
    /// The interpolated vertex normal as colour, with no lighting at all.
    Normals,
    Wire,
    ShadedWire,
    /// The UV checker over UV set 0, lit: what texture swim and stretching look like.
    Checker,
}

impl Look {
    pub fn name(self) -> &'static str {
        match self {
            Look::Shaded => "shaded",
            Look::Matte => "matte",
            Look::Normals => "normals",
            Look::Wire => "wire",
            Look::ShadedWire => "shaded-wire",
            Look::Checker => "checker",
        }
    }
}

pub fn parse(mut arguments: impl Iterator<Item = String>) -> Result<Option<Args>> {
    let mut args = Args {
        inputs: Vec::new(),
        out: PathBuf::from("target/snapshots"),
        name: "snap".into(),
        size: (800, 600),
        views: Vec::new(),
        looks: Vec::new(),
        frame_on: None,
        chrome: [0; 4],
        background: ViewportBackground::Grey25,
        grid: false,
        compare: false,
        singles: false,
        inspect: false,
        render: true,
        clip: None,
        attach: Vec::new(),
        curves: false,
        skeleton: false,
        select: Vec::new(),
    };
    while let Some(argument) = arguments.next() {
        let mut value = |flag: &str| {
            arguments
                .next()
                .ok_or_else(|| anyhow!("{flag} needs a value"))
        };
        match argument.as_str() {
            "-h" | "--help" => return Ok(None),
            "--out" => args.out = value("--out")?.into(),
            "--name" => args.name = value("--name")?,
            "--size" => args.size = parse_size(&value("--size")?)?,
            "--views" => {
                for view in value("--views")?
                    .split(';')
                    .filter(|v| !v.trim().is_empty())
                {
                    args.views.push(parse_view(view.trim())?);
                }
            }
            "--look" => {
                for look in value("--look")?.split(',') {
                    args.looks.push(parse_look(look.trim())?);
                }
            }
            "--frame" => {
                args.frame_on = Some(
                    value("--frame")?
                        .parse()
                        .context("--frame takes an index")?,
                )
            }
            "--chrome" => args.chrome = parse_chrome(&value("--chrome")?)?,
            "--background" => args.background = parse_background(&value("--background")?)?,
            "--grid" => args.grid = true,
            "--compare" => args.compare = true,
            "--singles" => args.singles = true,
            "--inspect" => args.inspect = true,
            "--no-render" => args.render = false,
            "--clip" => args.clip = Some(parse_clip(&value("--clip")?)?),
            "--attach" => args.attach.push(value("--attach")?.into()),
            "--curves" => args.curves = true,
            "--skeleton" => args.skeleton = true,
            "--select" => args.select.extend(
                value("--select")?
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned),
            ),
            flag if flag.starts_with("--") => bail!("unknown option {flag}"),
            input => args.inputs.push(parse_input(input)?),
        }
    }
    if args.inputs.is_empty() {
        return Ok(None);
    }
    if args.curves && args.clip.is_none() {
        bail!("--curves draws a clip's motion: it needs --clip");
    }
    let [left, top, right, bottom] = args.chrome;
    if left + right >= args.size.0 || top + bottom >= args.size.1 {
        bail!(
            "--chrome {:?} covers the whole {}x{} view",
            args.chrome,
            args.size.0,
            args.size.1
        );
    }
    if args.views.is_empty() {
        args.views = vec![parse_view("iso")?, parse_view("iso-back")?];
    }
    if args.looks.is_empty() {
        args.looks.push(Look::Shaded);
    }
    Ok(Some(args))
}

fn parse_clip(text: &str) -> Result<ClipSpec> {
    let (index, frame) = match text.split_once(':') {
        Some((index, frame)) => (index, Some(frame)),
        None => (text, None),
    };
    Ok(ClipSpec {
        index: index.trim().parse().context("--clip takes a clip index")?,
        frame: frame
            .map(|frame| frame.trim().parse().context("--clip's frame is a number"))
            .transpose()?,
    })
}

fn parse_size(text: &str) -> Result<(u32, u32)> {
    let (width, height) = text
        .split_once(['x', 'X'])
        .ok_or_else(|| anyhow!("--size takes WxH, got {text}"))?;
    let size = (width.parse()?, height.parse()?);
    if size.0 < 16 || size.1 < 16 || size.0 > 8192 || size.1 > 8192 {
        bail!("--size {text} is out of range");
    }
    Ok(size)
}

fn parse_chrome(text: &str) -> Result<[u32; 4]> {
    let numbers: Vec<u32> = text
        .split(',')
        .map(|n| n.trim().parse::<u32>())
        .collect::<Result<_, _>>()
        .with_context(|| format!("--chrome takes left,top,right,bottom in pixels: {text}"))?;
    match numbers[..] {
        [left, top, right, bottom] => Ok([left, top, right, bottom]),
        _ => bail!("--chrome takes four numbers: {text}"),
    }
}

fn parse_view(text: &str) -> Result<View> {
    let named = |yaw: f32, pitch: f32| View {
        name: text.to_owned(),
        yaw_degrees: yaw,
        pitch_degrees: pitch,
        zoom: 1.0,
        focus: None,
    };
    // The viewer's home view is +45 yaw, -35.26 pitch: the eye up in the +X/+Z
    // octant looking down. Negative pitch is "from above".
    Ok(match text {
        "iso" => named(45.0, -35.264),
        "iso-back" => named(225.0, -35.264),
        "front" => named(0.0, -10.0),
        "back" => named(180.0, -10.0),
        "left" => named(-90.0, -10.0),
        "right" => named(90.0, -10.0),
        "top" => named(0.0, -85.0),
        "bottom" => named(0.0, 85.0),
        _ => {
            let numbers: Vec<f32> = text
                .split(',')
                .map(|n| n.trim().parse::<f32>())
                .collect::<Result<_, _>>()
                .with_context(|| {
                    format!("a view is a preset name or yaw,pitch[,zoom[,x,y,z]]: {text}")
                })?;
            if !matches!(numbers.len(), 2 | 3 | 6) {
                bail!("a view is yaw,pitch[,zoom[,x,y,z]]: {text}");
            }
            View {
                name: text.to_owned(),
                yaw_degrees: numbers[0],
                pitch_degrees: numbers[1],
                zoom: numbers.get(2).copied().unwrap_or(1.0).max(0.01),
                focus: (numbers.len() == 6).then(|| [numbers[3], numbers[4], numbers[5]]),
            }
        }
    })
}

fn parse_look(text: &str) -> Result<Look> {
    Ok(match text {
        "shaded" => Look::Shaded,
        "matte" => Look::Matte,
        "normals" => Look::Normals,
        "wire" => Look::Wire,
        "shaded-wire" => Look::ShadedWire,
        "checker" => Look::Checker,
        _ => bail!("unknown look {text}"),
    })
}

fn parse_background(text: &str) -> Result<ViewportBackground> {
    Ok(match text {
        "black" => ViewportBackground::Black,
        "grey25" => ViewportBackground::Grey25,
        "grey50" => ViewportBackground::Grey50,
        "grey75" => ViewportBackground::Grey75,
        "white" => ViewportBackground::White,
        "gradient" => ViewportBackground::Gradient,
        _ => bail!("unknown background {text}"),
    })
}

/// `path[@stack][#level]`. The suffixes are looked for only after the file's
/// extension, so a `#` or `@` in a directory name is not mistaken for one.
fn parse_input(text: &str) -> Result<InputSpec> {
    let lower = text.to_ascii_lowercase();
    let path_end = lower
        .find(".fbx")
        .map(|at| at + 4)
        .ok_or_else(|| anyhow!("an input must name an .fbx file: {text}"))?;
    let (path, mut rest) = text.split_at(path_end);
    let mut level = LevelSpec::Default;
    if let Some((before, after)) = rest.split_once('#') {
        level = match after.to_ascii_lowercase().as_str() {
            "all" | "lod*" => LevelSpec::All,
            selector => LevelSpec::One(
                selector
                    .trim_start_matches("lod")
                    .parse()
                    .with_context(|| format!("unknown level selector #{after}"))?,
            ),
        };
        rest = before;
    }
    let stack = match rest.strip_prefix('@') {
        None if rest.is_empty() => None,
        None => bail!("unexpected text after the file name: {rest}"),
        Some(lods) if lods == "lods" || lods.starts_with("lods:") || lods.starts_with("lods/") => {
            let (list, error) = match lods.split_once('/') {
                Some((list, error)) => (
                    list,
                    Some(
                        error
                            .parse::<f32>()
                            .with_context(|| format!("@lods error limit: {error}"))?,
                    ),
                ),
                None => (lods, None),
            };
            let ratios = match list.strip_prefix("lods:") {
                Some(list) => list
                    .split(',')
                    .map(|r| r.trim().parse::<f32>())
                    .collect::<Result<Vec<_>, _>>()
                    .with_context(|| format!("@lods takes ratios: {lods}"))?,
                None => vec![0.5, 0.25, 0.125],
            };
            Some(StackSpec::Lods(ratios, error))
        }
        Some(preset) => Some(StackSpec::Preset(preset.into())),
    };
    Ok(InputSpec {
        path: path.into(),
        stack,
        level,
    })
}

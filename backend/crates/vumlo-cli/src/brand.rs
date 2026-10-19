//! `vumlo-cli brand apply`: everything the web build takes from the institution's configuration.
//!
//! It writes `.vumlo/generated/` next to `vumlo.toml`, from scratch every time: the public part of
//! the configuration as `client-config.json` and its TypeScript types, the JSON Schema of
//! `vumlo.toml`, the web manifest, and from the logo the favicon and the app icons. The web build
//! reads only this directory (ADR-0054). Nothing here is served at run time: changing the
//! configuration means `brand apply`, a web rebuild and a restart.

use std::io::Cursor;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, bail};
use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::codecs::png::PngEncoder;
use image::imageops::{self, FilterType};
use image::{ExtendedColorType, ImageEncoder, Rgba, RgbaImage};
use module_api::OutboundClient;
use serde_json::json;
use vumlo_config::{ClientConfig, Options, Settings};

use crate::runtime::{self, VERSION};

/// The most a logo may weigh, as a file or a download.
const LOGO_MAX_BYTES: usize = 5 * 1024 * 1024;
const LOGO_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10);
/// A raster logo may not be larger than this in either direction (a decompression bomb guard).
const LOGO_MAX_SIDE: u32 = 4096;
/// Every icon is scaled from this one. 1024 is what the desktop app's installer icons are made from (ADR-0062).
const MASTER: u32 = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogoKind {
    Png,
    Jpeg,
    Webp,
    Svg,
}

impl LogoKind {
    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
            Self::Svg => "svg",
        }
    }
}

pub struct Logo {
    pub bytes: Vec<u8>,
    pub kind: LogoKind,
}

/// The files of `.vumlo/generated/`, by name, ready to write.
pub type Files = Vec<(String, Vec<u8>)>;

pub async fn apply(options: &Options, dry_run: bool) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let settings = &loaded.settings;
    let files = generate_from_config(settings).await?;
    let out = generated_dir(settings);
    if dry_run {
        for (name, bytes) in &files {
            println!("would write  {name} ({} bytes)", bytes.len());
        }
        println!("into {}", out.display());
        return Ok(());
    }
    write_tree(&out, &files)?;
    println!("wrote {} files to {}", files.len(), out.display());
    if settings.institution.logo.is_none() {
        println!("(no institution.logo: no favicon or icons were generated)");
    }
    println!("Next: rebuild the web app (`vumlo-cli build web`) and restart.");
    Ok(())
}

/// Where the generated files go: next to the config file.
pub fn generated_dir(settings: &Settings) -> std::path::PathBuf {
    settings.base_dir.join(".vumlo/generated")
}

/// Loads the logo the configuration names and renders everything from it.
pub async fn generate_from_config(settings: &Settings) -> anyhow::Result<Files> {
    let logo = match &settings.institution.logo {
        Some(source) => Some(load_logo(settings, source).await?),
        None => None,
    };
    generate(settings, logo.as_ref())
}

/// A problem with `institution.logo` that is visible without fetching it: for `config check`.
pub fn logo_problem(settings: &Settings) -> Option<String> {
    let source = settings.institution.logo.as_deref()?;
    if is_url(source) {
        return None;
    }
    let path = settings.base_dir.join(source);
    (!path.is_file()).then(|| {
        format!(
            "institution.logo: {} is not a file (a path relative to vumlo.toml, or an http(s) URL)",
            path.display()
        )
    })
}

fn is_url(source: &str) -> bool {
    source.starts_with("http://") || source.starts_with("https://")
}

async fn load_logo(settings: &Settings, source: &str) -> anyhow::Result<Logo> {
    let bytes = if is_url(source) {
        let client = OutboundClient::new(
            LOGO_DOWNLOAD_TIMEOUT,
            LOGO_DOWNLOAD_TIMEOUT,
            LOGO_MAX_BYTES,
            3,
        )?;
        let (status, body) = client
            .get(source)
            .await
            .with_context(|| "cannot download institution.logo")?;
        if status != 200 {
            bail!("institution.logo: the URL answered HTTP {status}");
        }
        body
    } else {
        let path = settings.base_dir.join(source);
        let size = std::fs::metadata(&path)
            .with_context(|| format!("institution.logo: cannot read {}", path.display()))?
            .len();
        if size > LOGO_MAX_BYTES as u64 {
            bail!("institution.logo: {} is larger than 5 MiB", path.display());
        }
        std::fs::read(&path)
            .with_context(|| format!("institution.logo: cannot read {}", path.display()))?
    };
    let kind = detect(&bytes).context(
        "institution.logo: not a PNG, JPEG, WebP or SVG image (SVG text must be converted to paths)",
    )?;
    Ok(Logo { bytes, kind })
}

fn detect(bytes: &[u8]) -> Option<LogoKind> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some(LogoKind::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(LogoKind::Jpeg)
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(LogoKind::Webp)
    } else if std::str::from_utf8(&bytes[..bytes.len().min(2048)])
        .is_ok_and(|head| head.contains("<svg"))
    {
        Some(LogoKind::Svg)
    } else {
        None
    }
}

/// The generated files, deterministic: the same configuration and logo give the same bytes.
pub fn generate(settings: &Settings, logo: Option<&Logo>) -> anyhow::Result<Files> {
    let mut files: Files = Vec::new();
    let logo_name = logo.map(|l| format!("logo.{}", l.kind.extension()));

    let client = ClientConfig::new(settings, VERSION, logo_name.clone());
    files.push(("client-config.json".into(), pretty(&json!(client))?));
    files.push((
        "client-config.d.ts".into(),
        vumlo_config::client_config_types()
            .map_err(|e| anyhow::anyhow!("cannot generate the TypeScript types: {e}"))?
            .into_bytes(),
    ));
    files.push((
        "vumlo.schema.json".into(),
        pretty(vumlo_config::settings_schema())?,
    ));

    let icons = |sizes: &[(&str, u32)]| -> Vec<serde_json::Value> {
        sizes
            .iter()
            .map(|(name, side)| {
                json!({"src": format!("/brand/{name}"), "sizes": format!("{side}x{side}"), "type": "image/png"})
            })
            .collect()
    };
    let manifest_icons = if logo.is_some() {
        icons(&[("icon-192.png", 192), ("icon-512.png", 512)])
    } else {
        Vec::new()
    };
    files.push((
        "site.webmanifest".into(),
        pretty(&json!({
            "name": settings.institution.name,
            "short_name": settings.institution.short_name,
            "lang": settings.institution.language.as_str(),
            "start_url": "/",
            "scope": "/",
            "display": "standalone",
            "icons": manifest_icons,
        }))?,
    ));

    if let (Some(logo), Some(name)) = (logo, logo_name) {
        let master = render_master(logo)?;
        files.push((name, logo.bytes.clone()));
        files.push(("icon-1024.png".into(), png(&master)?));
        files.push(("icon-512.png".into(), png(&scaled(&master, 512))?));
        files.push(("icon-192.png".into(), png(&scaled(&master, 192))?));
        files.push((
            "apple-touch-icon.png".into(),
            png(&on_white(&scaled(&master, 180)))?,
        ));
        files.push(("favicon.ico".into(), favicon(&master)?));
    }
    Ok(files)
}

fn pretty(value: &serde_json::Value) -> anyhow::Result<Vec<u8>> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    Ok(text.into_bytes())
}

/// The logo as a square `MASTER` x `MASTER` image with a transparent margin where the logo is not
/// square.
fn render_master(logo: &Logo) -> anyhow::Result<RgbaImage> {
    match logo.kind {
        LogoKind::Svg => render_svg(&logo.bytes),
        _ => {
            let mut reader = image::ImageReader::new(Cursor::new(&logo.bytes))
                .with_guessed_format()
                .context("cannot read institution.logo")?;
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(LOGO_MAX_SIDE);
            limits.max_image_height = Some(LOGO_MAX_SIDE);
            reader.limits(limits);
            let decoded = reader.decode().context(
                "institution.logo: cannot decode the image (or it is larger than 4096 px)",
            )?;
            Ok(fit_square(&decoded.to_rgba8(), MASTER))
        }
    }
}

fn render_svg(bytes: &[u8]) -> anyhow::Result<RgbaImage> {
    use resvg::tiny_skia::{Pixmap, Transform};
    let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default())
        .context("institution.logo: cannot read the SVG")?;
    let size = tree.size();
    let longest = size.width().max(size.height());
    if longest.is_nan() || longest <= 0.0 {
        bail!("institution.logo: the SVG has no size");
    }
    let scale = MASTER as f32 / longest;
    let mut pixmap = Pixmap::new(MASTER, MASTER).context("cannot allocate the icon")?;
    let transform = Transform::from_scale(scale, scale).post_translate(
        (MASTER as f32 - size.width() * scale) / 2.0,
        (MASTER as f32 - size.height() * scale) / 2.0,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let mut out = RgbaImage::new(MASTER, MASTER);
    for (target, source) in out.pixels_mut().zip(pixmap.pixels()) {
        let c = source.demultiply();
        *target = Rgba([c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Ok(out)
}

fn fit_square(source: &RgbaImage, side: u32) -> RgbaImage {
    let (w, h) = source.dimensions();
    let scale = side as f32 / w.max(h) as f32;
    let nw = ((w as f32 * scale).round() as u32).clamp(1, side);
    let nh = ((h as f32 * scale).round() as u32).clamp(1, side);
    let resized = imageops::resize(source, nw, nh, FilterType::Lanczos3);
    let mut canvas = RgbaImage::from_pixel(side, side, Rgba([0, 0, 0, 0]));
    imageops::overlay(
        &mut canvas,
        &resized,
        i64::from((side - nw) / 2),
        i64::from((side - nh) / 2),
    );
    canvas
}

fn scaled(master: &RgbaImage, side: u32) -> RgbaImage {
    imageops::resize(master, side, side, FilterType::Lanczos3)
}

/// iOS fills transparency with black, so the touch icon gets a white background.
fn on_white(icon: &RgbaImage) -> RgbaImage {
    let mut canvas = RgbaImage::from_pixel(icon.width(), icon.height(), Rgba([255, 255, 255, 255]));
    imageops::overlay(&mut canvas, icon, 0, 0);
    canvas
}

fn png(image: &RgbaImage) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            ExtendedColorType::Rgba8,
        )
        .context("cannot encode a PNG")?;
    Ok(out)
}

fn favicon(master: &RgbaImage) -> anyhow::Result<Vec<u8>> {
    let mut frames = Vec::new();
    for side in [16, 32, 48] {
        let icon = scaled(master, side);
        frames.push(
            IcoFrame::as_png(icon.as_raw(), side, side, ExtendedColorType::Rgba8)
                .context("cannot encode the favicon")?,
        );
    }
    let mut out = Vec::new();
    IcoEncoder::new(&mut out)
        .encode_images(&frames)
        .context("cannot encode the favicon")?;
    Ok(out)
}

/// Replaces `out` with exactly `files`: they are written next to it first, so a failure half way
/// leaves the previous directory as it was.
pub fn write_tree(out: &Path, files: &Files) -> anyhow::Result<()> {
    let parent = out
        .parent()
        .context("the generated directory has no parent")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("cannot create {}", parent.display()))?;
    let staging = parent.join("generated.tmp");
    if staging.exists() {
        std::fs::remove_dir_all(&staging)
            .with_context(|| format!("cannot clear {}", staging.display()))?;
    }
    std::fs::create_dir(&staging)
        .with_context(|| format!("cannot create {}", staging.display()))?;
    for (name, bytes) in files {
        std::fs::write(staging.join(name), bytes)
            .with_context(|| format!("cannot write {name}"))?;
    }
    if out.exists() {
        std::fs::remove_dir_all(out)
            .with_context(|| format!("cannot replace {}", out.display()))?;
    }
    std::fs::rename(&staging, out)
        .with_context(|| format!("cannot move the new files to {}", out.display()))?;
    Ok(())
}

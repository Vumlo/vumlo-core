//! `vumlo-cli brand apply`: everything the web build takes from the installation's configuration.
//!
//! It writes straight into the web app (`web.dir`), from scratch every time: the public part of
//! the configuration as `src/generated/client-config.json` and its TypeScript types, and in
//! `public/brand/` the web manifest, the favicon and the app icons made from the Vumlo mark.
//! The logo is Vumlo's, the same in every installation (ADR-0054, amended). The web build and
//! the desktop build read only these files. Nothing here is served at run time: changing the
//! configuration means `brand apply`, a web rebuild and a restart. An installation without a web
//! app (backend only) has nothing to brand.

use std::io::Cursor;
use std::path::Path;

use anyhow::Context;
use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::codecs::png::PngEncoder;
use image::imageops::{self, FilterType};
use image::{ExtendedColorType, ImageEncoder, Rgba, RgbaImage};
use serde_json::json;
use vumlo_config::{ClientConfig, Options, Settings};

use crate::runtime::{self, VERSION};

/// The Vumlo mark (`brand/vumlo-mark.png`), compiled in: every installation has the same one.
const LOGO: &[u8] = include_bytes!("../../../../brand/vumlo-mark.png");
/// Every icon is scaled from this one. 1024 is what the desktop app's installer icons are made from (ADR-0062).
const MASTER: u32 = 1024;

/// The files of one directory, by name, ready to write.
pub type Files = Vec<(String, Vec<u8>)>;

/// What `brand apply` writes: the client config for the web app's source, and the public assets.
pub struct Generated {
    /// `<web>/src/generated/`
    pub client: Files,
    /// `<web>/public/brand/`
    pub brand: Files,
}

pub fn apply(options: &Options, dry_run: bool) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let settings = &loaded.settings;
    let web = runtime::from_config_dir(settings, &settings.web.dir);
    if !web.join("package.json").is_file() {
        println!(
            "brand has nothing to do: {} has no web app (a backend-only installation)",
            web.display()
        );
        return Ok(());
    }
    let generated = generate(settings)?;
    let targets = [
        (web.join("src/generated"), &generated.client),
        (web.join("public/brand"), &generated.brand),
    ];
    if dry_run {
        for (out, files) in &targets {
            for (name, bytes) in *files {
                println!("would write  {name} ({} bytes)", bytes.len());
            }
            println!("into {}", out.display());
        }
        return Ok(());
    }
    for (out, files) in &targets {
        write_tree(out, files)?;
        println!("wrote {} files to {}", files.len(), out.display());
    }
    println!("Next: rebuild the web app (`vumlo-cli build web`) and restart.");
    Ok(())
}

/// The name of the app: "Vumlo | Northfield", or just "Vumlo" when the short name is Vumlo itself.
fn app_name(settings: &Settings) -> String {
    let short = &settings.institution.short_name;
    if short.eq_ignore_ascii_case("vumlo") {
        "Vumlo".to_owned()
    } else {
        format!("Vumlo | {short}")
    }
}

/// The generated files, deterministic: the same configuration gives the same bytes.
pub fn generate(settings: &Settings) -> anyhow::Result<Generated> {
    let client_config = ClientConfig::new(settings, VERSION);
    let client: Files = vec![
        ("client-config.json".into(), pretty(&json!(client_config))?),
        (
            "client-config.d.ts".into(),
            vumlo_config::client_config_types()
                .map_err(|e| anyhow::anyhow!("cannot generate the TypeScript types: {e}"))?
                .into_bytes(),
        ),
    ];
    let mut files: Files = Vec::new();

    let icons: Vec<serde_json::Value> = [("icon-192.png", 192), ("icon-512.png", 512)]
        .iter()
        .map(|(name, side)| {
            json!({"src": format!("/brand/{name}"), "sizes": format!("{side}x{side}"), "type": "image/png"})
        })
        .collect();
    files.push((
        "site.webmanifest".into(),
        pretty(&json!({
            "name": app_name(settings),
            "short_name": "Vumlo",
            "lang": settings.institution.language.as_str(),
            "start_url": "/",
            "scope": "/",
            "display": "standalone",
            "icons": icons,
        }))?,
    ));

    let master = render_master()?;
    files.push(("icon-1024.png".into(), png(&master)?));
    files.push(("icon-512.png".into(), png(&scaled(&master, 512))?));
    files.push(("icon-192.png".into(), png(&scaled(&master, 192))?));
    files.push((
        "apple-touch-icon.png".into(),
        png(&on_white(&scaled(&master, 180)))?,
    ));
    files.push(("favicon.ico".into(), favicon(&master)?));
    Ok(Generated {
        client,
        brand: files,
    })
}

fn pretty(value: &serde_json::Value) -> anyhow::Result<Vec<u8>> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    Ok(text.into_bytes())
}

/// The logo as a square `MASTER` x `MASTER` image with a transparent margin where it is not square.
fn render_master() -> anyhow::Result<RgbaImage> {
    let decoded = image::ImageReader::new(Cursor::new(LOGO))
        .with_guessed_format()
        .and_then(|reader| reader.decode().map_err(std::io::Error::other))
        .context("cannot read brand/vumlo-mark.png")?;
    Ok(fit_square(&decoded.to_rgba8(), MASTER))
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
    let parent = out.parent().context("the output directory has no parent")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("cannot create {}", parent.display()))?;
    let staging = out.with_extension("tmp");
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

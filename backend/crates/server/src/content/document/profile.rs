//! Profiles: the set of features a place of the editor allows (ADR-0025, ADR-0027).
//!
//! The profiles live in code; the institution's config can only narrow them. The effective
//! profile is the code profile cut down by `editor.profiles.<name>.features`, and every profile
//! loses `video` when `editor.video_providers` is empty.
//!
//! Later tasks add to the code profiles (ADR-0025):
//!
//! | feature      | full | question | option | student |
//! |--------------|:----:|:--------:|:------:|:-------:|
//! | image        |  ✓   |    ✓     |   ✓    |    ✓    |
//! | attachments  |  ✓   |          |        |    ✓    |
//! | html         | ✓ (only with `editor.html`) | | |  |

use std::collections::BTreeSet;

use api_types::{
    Block, EditorProfile, EditorProfiles, Feature, Inline, LinkRun, ProfileName, TextRun,
    VideoProvider,
};

use super::{DocumentError, MAX_ERRORS, limits};
use vumlo_config::EditorSettings;

/// The editor configuration cannot be applied. The message names the key and the value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct EditorConfigError(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveProfile {
    pub name: ProfileName,
    pub features: BTreeSet<Feature>,
    /// A document of this profile is exactly one block, without children.
    pub single_block: bool,
}

/// The four profiles after the config narrowed them, and the enabled video providers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveProfiles {
    profiles: [EffectiveProfile; 4],
    video_providers: BTreeSet<VideoProvider>,
}

fn index(name: ProfileName) -> usize {
    match name {
        ProfileName::Full => 0,
        ProfileName::Question => 1,
        ProfileName::Option => 2,
        ProfileName::Student => 3,
    }
}

fn code_profile(name: ProfileName) -> EffectiveProfile {
    let all = || Feature::ALL.iter().copied();
    let features = match name {
        ProfileName::Full | ProfileName::Question => all().collect(),
        ProfileName::Option => [
            Feature::TextStyles,
            Feature::Links,
            Feature::Math,
            Feature::Code,
        ]
        .into(),
        ProfileName::Student => all().filter(|f| *f != Feature::Video).collect(),
    };
    EffectiveProfile {
        name,
        features,
        single_block: name == ProfileName::Option,
    }
}

fn names<T: Copy>(all: &[T], name: fn(T) -> &'static str) -> String {
    all.iter()
        .map(|&item| name(item))
        .collect::<Vec<_>>()
        .join(", ")
}

impl EffectiveProfiles {
    /// The profiles as the code defines them, with every provider enabled.
    pub fn unrestricted() -> Self {
        Self {
            profiles: [
                code_profile(ProfileName::Full),
                code_profile(ProfileName::Question),
                code_profile(ProfileName::Option),
                code_profile(ProfileName::Student),
            ],
            video_providers: VideoProvider::ALL.iter().copied().collect(),
        }
    }

    /// Applies the config. It can only narrow: an unknown profile, feature or provider, or a
    /// feature the profile does not have, is an error that names it.
    pub fn resolve(settings: &EditorSettings) -> Result<Self, EditorConfigError> {
        let mut resolved = Self::unrestricted();
        for (key, profile_settings) in &settings.profiles {
            let Some(name) = ProfileName::from_name(key) else {
                return Err(EditorConfigError(format!(
                    "editor.profiles.{key}: unknown profile (expected one of: {})",
                    names(ProfileName::ALL, ProfileName::as_str)
                )));
            };
            let allowed = code_profile(name).features;
            let mut chosen = BTreeSet::new();
            for text in &profile_settings.features {
                let Some(feature) = Feature::from_name(text) else {
                    return Err(EditorConfigError(format!(
                        "editor.profiles.{key}.features: unknown feature `{text}` (expected one of: {})",
                        names(Feature::ALL, Feature::as_str)
                    )));
                };
                if !allowed.contains(&feature) {
                    return Err(EditorConfigError(format!(
                        "editor.profiles.{key}.features: `{text}` is not a feature of the {key} profile; the config can only narrow a profile"
                    )));
                }
                chosen.insert(feature);
            }
            resolved.profiles[index(name)].features = chosen;
        }
        let mut providers = BTreeSet::new();
        for text in &settings.video_providers {
            let Some(provider) = VideoProvider::from_name(text) else {
                return Err(EditorConfigError(format!(
                    "editor.video_providers: unknown provider `{text}` (expected one of: {})",
                    names(VideoProvider::ALL, VideoProvider::as_str)
                )));
            };
            providers.insert(provider);
        }
        if providers.is_empty() {
            for profile in &mut resolved.profiles {
                profile.features.remove(&Feature::Video);
            }
        }
        resolved.video_providers = providers;
        Ok(resolved)
    }

    pub fn profile(&self, name: ProfileName) -> &EffectiveProfile {
        &self.profiles[index(name)]
    }

    pub fn video_providers(&self) -> &BTreeSet<VideoProvider> {
        &self.video_providers
    }

    /// What `GET /editor/profiles` returns.
    pub fn to_dto(&self) -> EditorProfiles {
        EditorProfiles {
            profiles: self
                .profiles
                .iter()
                .map(|profile| EditorProfile {
                    name: profile.name,
                    features: profile.features.iter().copied().collect(),
                    single_block: profile.single_block,
                    limits: limits(),
                })
                .collect(),
            video_providers: self.video_providers.iter().copied().collect(),
        }
    }
}

fn at(path: &str, key: &str) -> String {
    format!("{path}.{key}")
}

fn nth(path: &str, index: usize) -> String {
    format!("{path}[{index}]")
}

/// Checks a parsed document against a profile; the paths match the ones the parser reports.
pub(super) fn check_profile(
    document: &api_types::Document,
    profile: &EffectiveProfile,
    profiles: &EffectiveProfiles,
) -> Vec<DocumentError> {
    let mut check = Check {
        profile,
        providers: profiles.video_providers(),
        errors: Vec::new(),
    };
    if profile.single_block && document.blocks.len() != 1 {
        check.err("blocks", "must hold exactly one block");
    }
    check.blocks(&document.blocks, "blocks");
    check.errors
}

struct Check<'a> {
    profile: &'a EffectiveProfile,
    providers: &'a BTreeSet<VideoProvider>,
    errors: Vec<DocumentError>,
}

impl Check<'_> {
    fn err(&mut self, path: &str, message: &'static str) {
        if self.errors.len() < MAX_ERRORS {
            self.errors.push(DocumentError {
                path: path.to_owned(),
                message,
            });
        }
    }

    /// True when the profile has `feature`; otherwise records an error at `path`.
    fn needs(&mut self, feature: Feature, path: &str) -> bool {
        let allowed = self.profile.features.contains(&feature);
        if !allowed {
            self.err(path, "is not allowed here");
        }
        allowed
    }

    fn blocks(&mut self, blocks: &[Block], path: &str) {
        for (index, block) in blocks.iter().enumerate() {
            self.block(block, &nth(path, index));
        }
    }

    fn block(&mut self, block: &Block, path: &str) {
        match block {
            Block::Paragraph { .. } => {}
            Block::Quote { .. } => {
                self.needs(Feature::Quote, path);
            }
            Block::Heading { .. } => {
                self.needs(Feature::Headings, path);
            }
            Block::BulletedListItem { .. }
            | Block::NumberedListItem { .. }
            | Block::CheckListItem { .. } => {
                self.needs(Feature::Lists, path);
            }
            Block::Divider { .. } => {
                self.needs(Feature::Divider, path);
            }
            Block::Table { props, .. } => {
                self.needs(Feature::Table, path);
                let rows_path = at(&at(path, "props"), "rows");
                for (r, row) in props.rows.iter().enumerate() {
                    for (c, cell) in row.iter().enumerate() {
                        self.inlines(cell, &nth(&nth(&rows_path, r), c));
                    }
                }
            }
            Block::Code { .. } => {
                self.needs(Feature::Code, path);
            }
            Block::Math { .. } => {
                self.needs(Feature::Math, path);
            }
            Block::Mermaid { .. } => {
                self.needs(Feature::Mermaid, path);
            }
            Block::Video { props, .. } => {
                if self.needs(Feature::Video, path) && !self.providers.contains(&props.provider) {
                    self.err(&at(&at(path, "props"), "provider"), "is not enabled");
                }
            }
        }
        if self.profile.single_block && !block.children().is_empty() {
            self.err(&at(path, "children"), "are not allowed here");
        }
        self.inlines(block.content(), &at(path, "content"));
        self.blocks(block.children(), &at(path, "children"));
    }

    fn inlines(&mut self, inlines: &[Inline], path: &str) {
        for (index, inline) in inlines.iter().enumerate() {
            let here = nth(path, index);
            match inline {
                Inline::Text(run) => self.styles(run, &here),
                Inline::Link { content, .. } => {
                    self.needs(Feature::Links, &here);
                    for (j, LinkRun::Text(run)) in content.iter().enumerate() {
                        self.styles(run, &nth(&at(&here, "content"), j));
                    }
                }
                Inline::Math { .. } => {
                    self.needs(Feature::Math, &here);
                }
            }
        }
    }

    fn styles(&mut self, run: &TextRun, path: &str) {
        if !run.styles.is_plain() {
            self.needs(Feature::TextStyles, &at(path, "styles"));
        }
    }
}

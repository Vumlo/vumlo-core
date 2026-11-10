//! The text the layer adds to a plugin's mail: the link and the footer, in the institution's
//! language (ADR-0085, ADR-0086). Account mail is composed by `auth` and sent as it is.

use vumlo_config::Language;

/// What `plugin_body` needs besides the text itself.
pub struct Footer<'a> {
    pub language: Language,
    pub institution: &'a str,
    pub plugin_id: &'a str,
    pub kind: &'a str,
    /// The absolute link of the signed opt-out route.
    pub opt_out_link: &'a str,
}

/// The plugin's text, then its link when it gave one, then the footer.
pub fn plugin_body(text: &str, link: Option<&str>, footer: &Footer<'_>) -> String {
    let mut body = text.trim_end().to_owned();
    if let Some(link) = link {
        body.push_str("\n\n");
        body.push_str(link);
    }
    body.push_str("\n\n-- \n");
    body.push_str(&footer_text(footer));
    body
}

fn footer_text(footer: &Footer<'_>) -> String {
    let Footer {
        language,
        institution,
        plugin_id,
        kind,
        opt_out_link,
    } = footer;
    match language {
        Language::En => format!(
            "This message was sent by the plugin {plugin_id} of {institution}.\nTo stop getting messages of the kind {kind} from this plugin, open:\n{opt_out_link}\n"
        ),
        Language::Uk => format!(
            "Це повідомлення надіслав плагін {plugin_id} закладу «{institution}».\nЩоб більше не отримувати від цього плагіна повідомлень типу {kind}, відкрийте:\n{opt_out_link}\n"
        ),
    }
}

/// The plain-text answer of the opt-out and opt-in routes.
pub fn opted_out_answer(language: Language, plugin_id: &str, opt_in_link: &str) -> String {
    match language {
        Language::En => format!(
            "You will not get messages of this kind from {plugin_id}.\nChanged your mind? Open:\n{opt_in_link}\n"
        ),
        Language::Uk => format!(
            "Ви більше не отримуватимете від {plugin_id} повідомлень цього типу.\nПередумали? Відкрийте:\n{opt_in_link}\n"
        ),
    }
}

pub fn opted_in_answer(language: Language, plugin_id: &str) -> String {
    match language {
        Language::En => format!("You will get messages of this kind from {plugin_id} again.\n"),
        Language::Uk => {
            format!("Ви знову отримуватимете від {plugin_id} повідомлення цього типу.\n")
        }
    }
}

#[cfg(test)]
mod tests;

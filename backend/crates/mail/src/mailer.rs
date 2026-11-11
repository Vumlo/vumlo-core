//! The transport (ADR-0045, moved here by ADR-0084): plain-text messages over SMTP, or into a
//! directory for development and tests. Built once from `[mail]`; a bad address or host stops the
//! server at startup.

use lettre::message::Mailbox;
use lettre::message::header::{ContentType, Header, HeaderName, HeaderValue};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncFileTransport, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::MailError;
use vumlo_config::{MailSettings, MailTransport, SmtpTls};

/// Why a message was not sent. The text is a class such as `smtp 451` or `connect`: the server's own
/// words can name the address, so they are dropped here and never stored or logged.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SendError {
    /// Worth trying again later: no connection, a timeout, an SMTP 4xx.
    #[error("{0}")]
    Transient(String),
    /// Trying again cannot help: an SMTP 5xx, an address the transport refuses, a message that
    /// cannot be built.
    #[error("{0}")]
    Permanent(String),
}

impl SendError {
    pub fn class(&self) -> &str {
        match self {
            Self::Transient(class) | Self::Permanent(class) => class,
        }
    }

    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::Permanent(_))
    }
}

macro_rules! plain_header {
    ($type:ident, $name:literal) => {
        #[derive(Clone)]
        struct $type(String);

        impl Header for $type {
            fn name() -> HeaderName {
                HeaderName::new_from_ascii_str($name)
            }

            fn parse(s: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
                Ok(Self(s.to_owned()))
            }

            fn display(&self) -> HeaderValue {
                HeaderValue::new(Self::name(), self.0.clone())
            }
        }
    };
}

plain_header!(ListUnsubscribe, "List-Unsubscribe");
plain_header!(ListUnsubscribePost, "List-Unsubscribe-Post");
plain_header!(XVumloPlugin, "X-Vumlo-Plugin");
plain_header!(AutoSubmitted, "Auto-Submitted");

/// One message to one person. The headers of a plugin's mail are added when `plugin` is set.
pub struct Outgoing<'a> {
    pub name: &'a str,
    pub address: &'a str,
    pub subject: &'a str,
    pub body: String,
    /// The plugin that wrote the mail, with the opt-out link for the `List-Unsubscribe` header.
    pub plugin: Option<(&'a str, &'a str)>,
}

enum Transport {
    Smtp(AsyncSmtpTransport<Tokio1Executor>),
    File(AsyncFileTransport<Tokio1Executor>),
}

pub struct Mailer {
    from: Mailbox,
    link_base: String,
    transport: Transport,
}

impl Mailer {
    /// `public_origin` is the default for `mail.link_base`.
    pub fn new(settings: &MailSettings, public_origin: &str) -> Result<Self, MailError> {
        let from = settings
            .from
            .parse::<Mailbox>()
            .map_err(|_| MailError::From(settings.from.clone()))?;
        let transport = match &settings.transport {
            MailTransport::File { dir } => {
                Transport::File(AsyncFileTransport::<Tokio1Executor>::new(dir))
            }
            MailTransport::Smtp {
                host,
                port,
                tls,
                username,
                password,
            } => {
                let bad_host = |_| MailError::Host(host.clone());
                let mut builder = match tls {
                    SmtpTls::Starttls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
                        .map_err(bad_host)?,
                    SmtpTls::Tls => {
                        AsyncSmtpTransport::<Tokio1Executor>::relay(host).map_err(bad_host)?
                    }
                    SmtpTls::Off => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host),
                };
                if let Some(port) = port {
                    builder = builder.port(*port);
                }
                if let (Some(user), Some(password)) = (username, password) {
                    builder = builder
                        .credentials(Credentials::new(user.clone(), password.expose().to_owned()));
                }
                Transport::Smtp(builder.build())
            }
        };
        Ok(Self {
            from,
            link_base: settings
                .link_base
                .clone()
                .unwrap_or_else(|| public_origin.to_owned()),
            transport,
        })
    }

    /// A link into the web app: `path` starts with `/`.
    pub fn link(&self, path: &str) -> String {
        format!("{}{path}", self.link_base)
    }

    pub async fn send(&self, outgoing: Outgoing<'_>) -> Result<(), SendError> {
        let permanent = |class: &str| SendError::Permanent(class.to_owned());
        let address = outgoing
            .address
            .parse::<lettre::Address>()
            .map_err(|_| permanent("address"))?;
        let mut builder = Message::builder()
            .from(self.from.clone())
            .to(Mailbox::new(Some(outgoing.name.to_owned()), address))
            .subject(outgoing.subject)
            .header(ContentType::TEXT_PLAIN);
        if let Some((plugin, unsubscribe)) = outgoing.plugin {
            builder = builder
                .header(ListUnsubscribe(format!("<{unsubscribe}>")))
                .header(ListUnsubscribePost("List-Unsubscribe=One-Click".to_owned()))
                .header(XVumloPlugin(plugin.to_owned()))
                .header(AutoSubmitted("auto-generated".to_owned()));
        }
        let message = builder
            .body(outgoing.body)
            .map_err(|_| permanent("build"))?;
        match &self.transport {
            Transport::Smtp(smtp) => smtp.send(message).await.map(|_| ()).map_err(|e| {
                let class = match e.status() {
                    Some(code) => format!("smtp {code}"),
                    None if e.is_timeout() => "timeout".to_owned(),
                    None if e.is_tls() => "tls".to_owned(),
                    None => "connect".to_owned(),
                };
                if e.is_permanent() {
                    SendError::Permanent(class)
                } else {
                    SendError::Transient(class)
                }
            }),
            Transport::File(file) => file
                .send(message)
                .await
                .map(|_| ())
                .map_err(|_| SendError::Transient("file".to_owned())),
        }
    }
}

#[cfg(test)]
mod tests;

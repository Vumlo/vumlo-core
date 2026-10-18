//! Outgoing mail (ADR-0045): plain-text messages over SMTP, or into a directory for development
//! and tests. Built once from `[mail]`; a bad address or host stops the server at startup.

use lettre::message::{Mailbox, header::ContentType};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncFileTransport, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use vumlo_config::{MailSettings, MailTransport, SmtpTls};

#[derive(Debug, thiserror::Error)]
pub enum MailError {
    #[error("mail.from \"{0}\" is not a mailbox such as \"Vumlo <no-reply@example.edu>\"")]
    From(String),
    #[error("mail.transport.host \"{0}\" cannot be used as an SMTP host")]
    Host(String),
    #[error("the recipient address is not valid")]
    Recipient,
    #[error("cannot build the message: {0}")]
    Message(#[from] lettre::error::Error),
    /// The server refused or could not be reached. The text comes from the transport and holds no
    /// message content.
    #[error("cannot send the message: {0}")]
    Send(String),
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

    pub async fn send(
        &self,
        to_name: &str,
        to_address: &str,
        subject: &str,
        body: String,
    ) -> Result<(), MailError> {
        let address = to_address
            .parse::<lettre::Address>()
            .map_err(|_| MailError::Recipient)?;
        let message = Message::builder()
            .from(self.from.clone())
            .to(Mailbox::new(Some(to_name.to_owned()), address))
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body)?;
        match &self.transport {
            Transport::Smtp(smtp) => smtp
                .send(message)
                .await
                .map(|_| ())
                .map_err(|e| MailError::Send(e.to_string())),
            Transport::File(file) => file
                .send(message)
                .await
                .map(|_| ())
                .map_err(|e| MailError::Send(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests;

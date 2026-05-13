//! Email sending. Implements allowthem's `EmailSender` trait so that
//! `ath.db().send_password_reset(...)` and friends can deliver mail.
//!
//! Production path is SMTP via `lettre`. When SMTP env is not configured the
//! factory falls back to `LogEmailSender`, which writes the message to the
//! tracing log — fine for local dev, never fine in prod.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use allowthem_core::{
    AuthError, EmailBranding, EmailMessage, EmailSender, LogEmailSender, render_email,
};
use lettre::message::{Mailbox, MultiPart, SinglePart, header::ContentType};
use lettre::transport::smtp::AsyncSmtpTransport;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncTransport, Message, Tokio1Executor};

/// How the SMTP client wraps the transport in TLS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpEncryption {
    /// Plain connection upgraded via STARTTLS (typical port 587).
    StartTls,
    /// Implicit TLS from the first byte (typical port 465).
    Tls,
    /// No TLS. Only appropriate for local test relays.
    None,
}

impl SmtpEncryption {
    fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "starttls" | "" => Some(Self::StartTls),
            "tls" | "implicit" => Some(Self::Tls),
            "none" | "plain" => Some(Self::None),
            _ => None,
        }
    }
}

/// SMTP relay configuration. Populate from env; all fields are required
/// except `port`, which defaults based on the encryption mode.
#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub from: String,
    pub encryption: SmtpEncryption,
}

impl SmtpConfig {
    /// Read SMTP config from env vars, or return `None` if `SUBSTRUKT_SMTP_HOST`
    /// is unset. A partially-configured env (host set but credentials missing)
    /// returns `Err` so a misconfigured prod deploy fails loudly instead of
    /// silently falling back to the log sender.
    pub fn from_env() -> eyre::Result<Option<Self>> {
        let Some(host) = std::env::var("SUBSTRUKT_SMTP_HOST").ok() else {
            return Ok(None);
        };

        let encryption = std::env::var("SUBSTRUKT_SMTP_ENCRYPTION")
            .ok()
            .as_deref()
            .map(SmtpEncryption::parse)
            .unwrap_or(Some(SmtpEncryption::StartTls))
            .ok_or_else(|| {
                eyre::eyre!("SUBSTRUKT_SMTP_ENCRYPTION must be one of: starttls, tls, none")
            })?;

        let default_port = match encryption {
            SmtpEncryption::StartTls => 587,
            SmtpEncryption::Tls => 465,
            SmtpEncryption::None => 25,
        };
        let port = match std::env::var("SUBSTRUKT_SMTP_PORT") {
            Ok(s) => s
                .parse::<u16>()
                .map_err(|_| eyre::eyre!("SUBSTRUKT_SMTP_PORT is not a valid port"))?,
            Err(_) => default_port,
        };

        let username = std::env::var("SUBSTRUKT_SMTP_USERNAME")
            .map_err(|_| eyre::eyre!("SUBSTRUKT_SMTP_USERNAME is required when SMTP is enabled"))?;
        let password = std::env::var("SUBSTRUKT_SMTP_PASSWORD")
            .map_err(|_| eyre::eyre!("SUBSTRUKT_SMTP_PASSWORD is required when SMTP is enabled"))?;
        let from = std::env::var("SUBSTRUKT_SMTP_FROM")
            .map_err(|_| eyre::eyre!("SUBSTRUKT_SMTP_FROM is required when SMTP is enabled"))?;

        Ok(Some(Self {
            host,
            port,
            username,
            password,
            from,
            encryption,
        }))
    }
}

/// SMTP-backed `EmailSender`. Cheap to clone; the underlying transport is
/// connection-pooled internally by lettre.
pub struct SmtpEmailSender {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
    branding: EmailBranding,
}

impl SmtpEmailSender {
    pub fn new(cfg: &SmtpConfig) -> eyre::Result<Self> {
        let from: Mailbox = cfg
            .from
            .parse()
            .map_err(|e| eyre::eyre!("SUBSTRUKT_SMTP_FROM is not a valid mailbox: {e}"))?;

        let creds = Credentials::new(cfg.username.clone(), cfg.password.clone());
        let builder = match cfg.encryption {
            SmtpEncryption::StartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host)?
            }
            SmtpEncryption::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.host)?,
            SmtpEncryption::None => {
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&cfg.host)
            }
        };

        let transport = builder.port(cfg.port).credentials(creds).build();
        Ok(Self {
            transport,
            from,
            branding: EmailBranding {
                app_name: "Substrukt".to_string(),
                logo_url: None,
                footer_line: None,
            },
        })
    }
}

impl EmailSender for SmtpEmailSender {
    fn send<'a>(
        &'a self,
        message: &'a EmailMessage,
    ) -> Pin<Box<dyn Future<Output = Result<(), AuthError>> + Send + 'a>> {
        Box::pin(async move {
            let rendered = render_email(&message.template, &self.branding);

            let to: Mailbox = message
                .to
                .parse()
                .map_err(|e: lettre::address::AddressError| AuthError::Email(e.to_string()))?;

            let builder = Message::builder()
                .from(self.from.clone())
                .to(to)
                .subject(&message.subject);

            let email = builder
                .multipart(
                    MultiPart::alternative()
                        .singlepart(
                            SinglePart::builder()
                                .header(ContentType::TEXT_PLAIN)
                                .body(rendered.text),
                        )
                        .singlepart(
                            SinglePart::builder()
                                .header(ContentType::TEXT_HTML)
                                .body(rendered.html),
                        ),
                )
                .map_err(|e| AuthError::Email(e.to_string()))?;

            self.transport
                .send(email)
                .await
                .map(|_| ())
                .map_err(|e| AuthError::Email(e.to_string()))
        })
    }
}

/// Build the sender for the running process.
///
/// Returns the SMTP sender when env is configured, otherwise the dev log
/// sender. Emits a warning on the log-sender path so a misconfigured prod
/// instance is noticeable.
pub fn build_sender(smtp: Option<SmtpConfig>) -> eyre::Result<Arc<dyn EmailSender>> {
    match smtp {
        Some(cfg) => {
            let sender = SmtpEmailSender::new(&cfg)?;
            tracing::info!(host = %cfg.host, port = cfg.port, "SMTP email sender configured");
            Ok(Arc::new(sender))
        }
        None => {
            tracing::warn!(
                "no SMTP env configured — using LogEmailSender; \
                 outgoing mail will only be logged, not delivered"
            );
            Ok(Arc::new(LogEmailSender))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn scoped_env<F: FnOnce()>(vars: &[(&str, Option<&str>)], f: F) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved: Vec<(String, Option<String>)> = vars
            .iter()
            .map(|(k, _)| (k.to_string(), std::env::var(k).ok()))
            .collect();
        for (k, v) in vars {
            match v {
                // Safety: tests run under `--test-threads=1` for this module
                // via the `serial` pattern; there is no parallel mutation.
                Some(val) => unsafe { std::env::set_var(k, val) },
                None => unsafe { std::env::remove_var(k) },
            }
        }
        f();
        for (k, old) in saved {
            match old {
                Some(v) => unsafe { std::env::set_var(&k, v) },
                None => unsafe { std::env::remove_var(&k) },
            }
        }
    }

    #[test]
    fn from_env_returns_none_when_host_unset() {
        scoped_env(&[("SUBSTRUKT_SMTP_HOST", None)], || {
            let cfg = SmtpConfig::from_env().unwrap();
            assert!(cfg.is_none());
        });
    }

    #[test]
    fn from_env_reads_full_starttls_config() {
        scoped_env(
            &[
                ("SUBSTRUKT_SMTP_HOST", Some("smtp.example.com")),
                ("SUBSTRUKT_SMTP_USERNAME", Some("u")),
                ("SUBSTRUKT_SMTP_PASSWORD", Some("p")),
                ("SUBSTRUKT_SMTP_FROM", Some("noreply@example.com")),
                ("SUBSTRUKT_SMTP_ENCRYPTION", None),
                ("SUBSTRUKT_SMTP_PORT", None),
            ],
            || {
                let cfg = SmtpConfig::from_env().unwrap().unwrap();
                assert_eq!(cfg.host, "smtp.example.com");
                assert_eq!(cfg.port, 587);
                assert_eq!(cfg.encryption, SmtpEncryption::StartTls);
                assert_eq!(cfg.from, "noreply@example.com");
            },
        );
    }

    #[test]
    fn from_env_errors_when_host_set_but_credentials_missing() {
        scoped_env(
            &[
                ("SUBSTRUKT_SMTP_HOST", Some("smtp.example.com")),
                ("SUBSTRUKT_SMTP_USERNAME", None),
                ("SUBSTRUKT_SMTP_PASSWORD", None),
                ("SUBSTRUKT_SMTP_FROM", None),
            ],
            || {
                let err = SmtpConfig::from_env().unwrap_err();
                let msg = err.to_string();
                assert!(msg.contains("SUBSTRUKT_SMTP_USERNAME"), "unexpected: {msg}");
            },
        );
    }

    #[test]
    fn encryption_parse_accepts_aliases() {
        assert_eq!(
            SmtpEncryption::parse("starttls"),
            Some(SmtpEncryption::StartTls)
        );
        assert_eq!(
            SmtpEncryption::parse("STARTTLS"),
            Some(SmtpEncryption::StartTls)
        );
        assert_eq!(SmtpEncryption::parse(""), Some(SmtpEncryption::StartTls));
        assert_eq!(SmtpEncryption::parse("tls"), Some(SmtpEncryption::Tls));
        assert_eq!(SmtpEncryption::parse("implicit"), Some(SmtpEncryption::Tls));
        assert_eq!(SmtpEncryption::parse("none"), Some(SmtpEncryption::None));
        assert_eq!(SmtpEncryption::parse("plain"), Some(SmtpEncryption::None));
        assert_eq!(SmtpEncryption::parse("nope"), None);
    }

    #[test]
    fn log_sender_is_the_fallback() {
        let sender = build_sender(None).unwrap();
        let fut = sender.send(EmailMessage {
            to: "a@b.com",
            subject: "hi",
            body: "hi",
            html: None,
        });
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(fut).unwrap();
    }
}

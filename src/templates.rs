use minijinja::Environment;
use minijinja_autoreload::AutoReloader;

/// Minijinja filter: format ISO 8601 timestamp as human-readable date.
/// Input: "2026-01-05T15:04:30.123456+00:00" or "2026-01-05T15:04:30Z"
/// Output: "Jan 5, 2026 3:04 PM"
/// Falls back to the original string if parsing fails.
fn datefmt(value: &str) -> String {
    // Try RFC 3339 first (most common format from chrono::Utc::now().to_rfc3339())
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
        return dt.format("%b %-d, %Y %-I:%M %p").to_string();
    }
    // Try ISO 8601 without timezone (SQLite datetime() format: "2026-01-05 15:04:30")
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S") {
        return dt.format("%b %-d, %Y %-I:%M %p").to_string();
    }
    // Try with fractional seconds and " UTC" suffix (allowthem's created_at.to_string() format)
    let trimmed = value.trim_end_matches(" UTC");
    if trimmed != value {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S%.f") {
            return dt.format("%b %-d, %Y %-I:%M %p").to_string();
        }
    }
    // Try just the date-time portion if it has a T separator but no timezone
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S") {
        return dt.format("%b %-d, %Y %-I:%M %p").to_string();
    }
    // Fall back to original string
    value.to_string()
}

/// Minijinja filter: format a JSON details string as human-readable key-value pairs.
/// Input: '{"name":"Production","slug":"prod"}' or any string
/// Output: "name: Production, slug: prod"
/// Falls back to the original string if JSON parsing fails or if it's not an object.
fn fmtdetails(value: &str) -> String {
    if let Ok(serde_json::Value::Object(map)) = serde_json::from_str(value) {
        let pairs: Vec<String> = map
            .iter()
            .map(|(k, v)| {
                let display = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                format!("{k}: {display}")
            })
            .collect();
        return pairs.join(", ");
    }
    value.to_string()
}

pub fn create_reloader() -> AutoReloader {
    AutoReloader::new(move |notifier| {
        let mut env = Environment::new();
        env.set_auto_escape_callback(minijinja::default_auto_escape_callback);

        // Debug: load from filesystem with hot-reload
        // Release: embed templates into the binary
        #[cfg(debug_assertions)]
        {
            env.set_loader(minijinja::path_loader("templates/"));
            notifier.set_fast_reload(true);
        }

        #[cfg(not(debug_assertions))]
        {
            let _ = notifier;
            minijinja_embed::load_templates!(&mut env);
        }

        // Default base_template -- overridden to "_partial.html" for htmx requests
        env.add_global("base_template", minijinja::Value::from("base.html"));

        env.add_filter("datefmt", datefmt);
        env.add_filter("fmtdetails", fmtdetails);

        Ok(env)
    })
}

/// Returns the base template name based on whether this is an htmx request.
pub fn base_for_htmx(is_htmx: bool) -> &'static str {
    if is_htmx {
        "_partial.html"
    } else {
        "base.html"
    }
}

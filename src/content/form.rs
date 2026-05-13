use std::collections::HashMap;

use serde_json::Value;

/// Maximum nesting depth for recursive schema traversal to prevent stack overflow.
const MAX_NESTING_DEPTH: usize = 32;

/// Map from field name to list of (id, label) pairs for reference dropdowns.
pub type ReferenceOptions = HashMap<String, Vec<(String, String)>>;

fn strip_array_indices(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    let mut in_bracket = false;
    for c in name.chars() {
        if c == '[' {
            in_bracket = true;
        } else if c == ']' {
            in_bracket = false;
        } else if !in_bracket {
            result.push(c);
        }
    }
    result
}

fn escape_html_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn build_hint_line(hints: &[String]) -> String {
    if hints.is_empty() {
        String::new()
    } else {
        format!(
            r#"  <p style="color: var(--fg-muted); font-size: 12px; margin-top: 4px;">{}</p>
"#,
            hints.join(" · ")
        )
    }
}

/// Extract `description` from a schema property, HTML-escaped.
fn get_description(schema: &Value) -> Option<String> {
    schema
        .get("description")
        .and_then(|d| d.as_str())
        .filter(|d| !d.is_empty())
        .map(|d| escape_html_attr(d))
}

/// Build string constraint HTML attrs and hint parts.
/// `is_textarea` controls whether `pattern` becomes an HTML attr or hint-only.
fn string_constraints(schema: &Value, is_textarea: bool) -> (String, Vec<String>) {
    let mut attrs = String::new();
    let mut hints = Vec::new();

    let min_len = schema
        .get("minLength")
        .and_then(|v| v.as_u64())
        .filter(|&v| v > 0);
    let max_len = schema.get("maxLength").and_then(|v| v.as_u64());

    if let Some(min) = min_len {
        attrs.push_str(&format!(r#" minlength="{min}""#));
    }
    if let Some(max) = max_len {
        attrs.push_str(&format!(r#" maxlength="{max}""#));
    }

    match (min_len, max_len) {
        (Some(min), Some(max)) => hints.push(format!("{min}–{max} characters")),
        (Some(min), None) => hints.push(format!("Min {min} characters")),
        (None, Some(max)) => hints.push(format!("Max {max} characters")),
        _ => {}
    }

    let pattern = schema
        .get("pattern")
        .and_then(|v| v.as_str())
        .filter(|p| !p.is_empty());
    if let Some(pat) = pattern {
        if !is_textarea {
            attrs.push_str(&format!(r#" pattern="{}""#, escape_html_attr(pat)));
        }
        hints.push(format!("Pattern: <code>{}</code>", escape_html_attr(pat)));
    }

    if schema.get("x-substrukt-unique").and_then(|v| v.as_bool()) == Some(true) {
        hints.push("Must be unique".to_string());
    }
    if schema
        .get("x-substrukt-required-if-published")
        .and_then(|v| v.as_bool())
        == Some(true)
    {
        hints.push("Required when published".to_string());
    }

    if let Some(desc) = get_description(schema) {
        hints.push(desc);
    }

    (attrs, hints)
}

/// Build number/integer constraint HTML attrs and hint parts.
fn number_constraints(schema: &Value, is_integer: bool) -> (String, Vec<String>) {
    let mut attrs = String::new();
    let mut hints = Vec::new();

    let minimum = schema.get("minimum").and_then(|v| v.as_f64());
    let maximum = schema.get("maximum").and_then(|v| v.as_f64());
    let exc_min = schema.get("exclusiveMinimum").and_then(|v| v.as_f64());
    let exc_max = schema.get("exclusiveMaximum").and_then(|v| v.as_f64());

    // Resolve effective min: tighter of minimum and exclusiveMinimum
    let (effective_min, min_exclusive) = match (minimum, exc_min) {
        (Some(m), Some(e)) => {
            let adj = if is_integer { e + 1.0 } else { e };
            if adj > m {
                (Some(adj), !is_integer)
            } else {
                (Some(m), false)
            }
        }
        (Some(m), None) => (Some(m), false),
        (None, Some(e)) => {
            if is_integer {
                (Some(e + 1.0), false)
            } else {
                (Some(e), true)
            }
        }
        (None, None) => (None, false),
    };

    // Resolve effective max: tighter of maximum and exclusiveMaximum
    let (effective_max, max_exclusive) = match (maximum, exc_max) {
        (Some(m), Some(e)) => {
            let adj = if is_integer { e - 1.0 } else { e };
            if adj < m {
                (Some(adj), !is_integer)
            } else {
                (Some(m), false)
            }
        }
        (Some(m), None) => (Some(m), false),
        (None, Some(e)) => {
            if is_integer {
                (Some(e - 1.0), false)
            } else {
                (Some(e), true)
            }
        }
        (None, None) => (None, false),
    };

    fn fmt_num(n: f64) -> String {
        if n.fract() == 0.0 {
            format!("{}", n as i64)
        } else {
            format!("{n}")
        }
    }

    if let Some(min) = effective_min {
        if !min_exclusive {
            attrs.push_str(&format!(r#" min="{}""#, fmt_num(min)));
        }
    }
    if let Some(max) = effective_max {
        if !max_exclusive {
            attrs.push_str(&format!(r#" max="{}""#, fmt_num(max)));
        }
    }

    // Hint text
    let min_hint = effective_min.map(|v| {
        if min_exclusive {
            format!("&gt; {}", fmt_num(v))
        } else {
            fmt_num(v)
        }
    });
    let max_hint = effective_max.map(|v| {
        if max_exclusive {
            format!("&lt; {}", fmt_num(v))
        } else {
            fmt_num(v)
        }
    });

    match (min_hint, max_hint) {
        (Some(min), Some(max)) => {
            if !min_exclusive && !max_exclusive {
                hints.push(format!("{min}–{max}"));
            } else {
                hints.push(format!("{min} to {max}"));
            }
        }
        (Some(min), None) => {
            if min_exclusive {
                hints.push(min);
            } else {
                hints.push(format!("Min {min}"));
            }
        }
        (None, Some(max)) => {
            if max_exclusive {
                hints.push(max);
            } else {
                hints.push(format!("Max {max}"));
            }
        }
        _ => {}
    }

    // multipleOf -> step
    if let Some(step) = schema.get("multipleOf").and_then(|v| v.as_f64()) {
        attrs.push_str(&format!(r#" step="{}""#, fmt_num(step)));
        hints.push(format!("Step: {}", fmt_num(step)));
    }

    if let Some(desc) = get_description(schema) {
        hints.push(desc);
    }

    (attrs, hints)
}

/// Generate HTML form fields from a JSON Schema.
pub fn render_form_fields(
    schema: &Value,
    data: Option<&Value>,
    prefix: &str,
    ref_options: &ReferenceOptions,
    app_slug: &str,
) -> String {
    render_form_fields_inner(schema, data, prefix, ref_options, app_slug, 0, 0, false)
}

/// Generate read-only HTML form fields from a JSON Schema.
pub fn render_form_fields_readonly(
    schema: &Value,
    data: Option<&Value>,
    prefix: &str,
    ref_options: &ReferenceOptions,
    app_slug: &str,
) -> String {
    render_form_fields_inner(schema, data, prefix, ref_options, app_slug, 0, 0, true)
}

fn render_form_fields_inner(
    schema: &Value,
    data: Option<&Value>,
    prefix: &str,
    ref_options: &ReferenceOptions,
    app_slug: &str,
    depth: usize,
    array_depth: usize,
    read_only: bool,
) -> String {
    if depth > MAX_NESTING_DEPTH {
        return r#"<div class="wf-field" style="color: var(--err); font-size: 13px;">Error: maximum nesting depth exceeded</div>"#.to_string();
    }

    let mut html = String::new();

    let properties = match schema.get("properties").and_then(|p| p.as_object()) {
        Some(p) => p,
        None => return html,
    };

    let required: Vec<&str> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    // Collect keys, ensuring "title" comes first
    let mut keys: Vec<&String> = properties.keys().collect();
    keys.sort_by_key(|k| if k.as_str() == "title" { 0 } else { 1 });

    for key in keys {
        let prop_schema = &properties[key];
        // Skip internal fields
        if key == "_id" || key == "_status" {
            continue;
        }

        let field_name = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        let label = prop_schema
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or(key);
        let is_required = required.contains(&key.as_str());
        let field_value = data.and_then(|d| d.get(key));
        let field_type = prop_schema
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("string");
        let format = prop_schema.get("format").and_then(|f| f.as_str());

        html.push_str(&render_field(
            &field_name,
            label,
            field_type,
            format,
            prop_schema,
            field_value,
            is_required,
            ref_options,
            app_slug,
            depth,
            array_depth,
            read_only,
        ));
    }

    html
}

fn render_field(
    name: &str,
    label: &str,
    field_type: &str,
    format: Option<&str>,
    schema: &Value,
    value: Option<&Value>,
    required: bool,
    ref_options: &ReferenceOptions,
    app_slug: &str,
    depth: usize,
    array_depth: usize,
    read_only: bool,
) -> String {
    let req_attr = if required { " required" } else { "" };
    let req_star = if required { " *" } else { "" };
    let req_msg = if required {
        r#"<span class="field-error">This field is required.</span>"#
    } else {
        ""
    };
    let readonly_attr = if read_only { " readonly" } else { "" };
    let disabled_attr = if read_only { " disabled" } else { "" };

    match (field_type, format) {
        ("string", Some("markdown")) => {
            let val = escape_html_attr(value.and_then(|v| v.as_str()).unwrap_or(""));
            let (constraint_attrs, hints) = string_constraints(schema, true);
            let hint_html = build_hint_line(&hints);
            let markdown_attr = if read_only { "" } else { " data-markdown" };
            format!(
                r#"<div class="wf-field" style="margin-top: 16px;">
  <label for="{name}" class="wf-label">{label}{req_star}</label>
  <textarea id="{name}" name="{name}" rows="12"{markdown_attr} class="wf-textarea" style="width: 100%; margin-top: 4px;"{constraint_attrs}{req_attr}{readonly_attr}>{val}</textarea>
{req_msg}{hint_html}</div>
"#
            )
        }
        ("string", Some("markdown-richtext")) => {
            let current_json = value
                .filter(|v| v.is_object())
                .map(|v| serde_json::to_string(v).unwrap_or_default())
                .unwrap_or_default();
            let preview_text = value
                .and_then(|v| v.get("markdown"))
                .and_then(|m| m.as_str())
                .map(|md| {
                    let plain: String = md
                        .chars()
                        .filter(|c| {
                            !matches!(
                                c,
                                '#' | '*' | '_' | '~' | '`' | '>' | '[' | ']' | '(' | ')' | '!'
                            )
                        })
                        .collect();
                    let trimmed = plain.trim();
                    if trimmed.len() > 200 {
                        format!("{}...", &trimmed[..200])
                    } else {
                        trimmed.to_string()
                    }
                })
                .unwrap_or_default();
            let snippet = if preview_text.is_empty() {
                "No content yet"
            } else {
                &preview_text
            };
            let escaped_json = escape_html_attr(&current_json);
            let desc = get_description(schema).map(|d| format!(r#"<p style="color: var(--fg-muted); font-size: 12px; margin-top: 4px;">{d}</p>"#)).unwrap_or_default();
            let open_button = if read_only {
                String::new()
            } else {
                r#"<button type="button" class="wf-btn primary" style="flex-shrink: 0;" data-richtext-open>Edit</button>"#.to_string()
            };
            let modal_html = if read_only {
                String::new()
            } else {
                format!(
                    r#"<div class="wf-overlay" id="richtext-overlay-{name}"></div>
  <div class="wf-modal wf-modal--lg" id="richtext-modal-{name}">
    <div class="wf-modal-head">
      <span class="wf-modal-title">EDIT: {label}</span>
      <div style="display: flex; gap: var(--space-2);">
        <button type="button" class="wf-btn" data-richtext-discard>Discard</button>
        <button type="button" class="wf-btn primary" data-richtext-save>Save &amp; Close</button>
      </div>
    </div>
    <div class="wf-modal-body" data-richtext-root style="padding: 0; flex: 1; overflow: auto;"></div>
  </div>"#
                )
            };
            let preview_cursor = if read_only { "default" } else { "pointer" };
            format!(
                r#"<div class="wf-field" style="margin-top: 16px;" data-richtext data-richtext-name="{name}" data-richtext-app="{app_slug}">
  <label class="wf-label">{label}{req_star}</label>
  <input type="hidden" name="{name}" value="{escaped_json}">
  <div data-richtext-preview style="display: flex; align-items: flex-start; gap: 12px; padding: 12px; border: var(--border-1) solid var(--hairline); margin-top: 4px; font-size: 13px; color: var(--fg-muted); cursor: {preview_cursor}; min-height: 48px;">
    <span data-richtext-snippet style="flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{snippet}</span>
    {open_button}
  </div>
  {desc}
  {modal_html}
{req_msg}</div>
"#
            )
        }
        ("string", Some("textarea")) => {
            let val = escape_html_attr(value.and_then(|v| v.as_str()).unwrap_or(""));
            let (constraint_attrs, hints) = string_constraints(schema, true);
            let hint_html = build_hint_line(&hints);
            format!(
                r#"<div class="wf-field" style="margin-top: 16px;">
  <label for="{name}" class="wf-label">{label}{req_star}</label>
  <textarea id="{name}" name="{name}" rows="6" class="wf-textarea" style="width: 100%; margin-top: 4px;"{constraint_attrs}{req_attr}{readonly_attr}>{val}</textarea>
{req_msg}{hint_html}</div>
"#
            )
        }
        ("string", Some("upload")) => {
            let mut current_html = String::new();
            if let Some(obj) = value.and_then(|v| v.as_object()) {
                let filename = escape_html_attr(
                    obj.get("filename")
                        .and_then(|f| f.as_str())
                        .unwrap_or("file"),
                );
                let hash = escape_html_attr(obj.get("hash").and_then(|h| h.as_str()).unwrap_or(""));
                let mime = escape_html_attr(obj.get("mime").and_then(|m| m.as_str()).unwrap_or(""));
                let json_val = escape_html_attr(
                    &serde_json::to_string(&value.unwrap_or(&Value::Null)).unwrap_or_default(),
                );

                let thumbnail = if mime.starts_with("image/") {
                    format!(
                        r#"<img src="/apps/{app_slug}/uploads/file/{hash}/{filename}" alt="{filename}" style="height: 64px; width: 64px; object-fit: cover; border: 1px solid var(--hairline);">"#
                    )
                } else {
                    String::new()
                };

                current_html = format!(
                    r#"<div style="margin-bottom: 8px; font-size: 13px; display: flex; align-items: center; gap: 12px;">
    {thumbnail}
    <div>
      <div>Current: <a href="/apps/{app_slug}/uploads/file/{hash}/{filename}" style="color: var(--accent); text-decoration: underline;" target="_blank">{filename}</a></div>
      <div style="color: var(--fg-muted); font-size: 12px;">{mime}</div>
    </div>
  </div>
  <input type="hidden" name="{name}.__current" value='{json_val}'>"#
                );
            }

            let hint_html =
                build_hint_line(&get_description(schema).into_iter().collect::<Vec<_>>());
            if read_only {
                let readonly_current = if current_html.is_empty() {
                    r#"<div style="color: var(--fg-muted); font-size: 13px; margin-top: 4px;">No file uploaded</div>"#.to_string()
                } else {
                    current_html
                };
                return format!(
                    r#"<div class="wf-field" style="margin-top: 16px;">
  <label class="wf-label">{label}{req_star}</label>
  {readonly_current}
{hint_html}</div>
"#
                );
            }
            format!(
                r#"<div class="wf-field" style="margin-top: 16px;">
  <label for="{name}" class="wf-label">{label}{req_star}</label>
  {current_html}
  <label class="wf-dropzone" data-upload-zone>
    <input type="file" id="{name}" name="{name}" class="wf-dropzone-input" data-upload-input{req_attr}>
    <div class="wf-dropzone-frame">
      <div class="wf-dropzone-glyph upload-zone-glyph">&darr;</div>
      <div class="wf-dropzone-title upload-zone-title">Drop files or click</div>
      <div class="wf-dropzone-hint upload-zone-hint">Any file type accepted</div>
    </div>
  </label>
{hint_html}</div>
"#
            )
        }
        ("string", Some("reference")) => {
            let val = value.and_then(|v| v.as_str()).unwrap_or("");
            let empty_opts = Vec::new();
            let stripped_name = strip_array_indices(name);
            let options = ref_options
                .get(name)
                .or_else(|| ref_options.get(&stripped_name))
                .unwrap_or(&empty_opts);
            let mut opts_html = r#"<option value="">-- Select --</option>"#.to_string();
            for (id, label_text) in options {
                let selected = if id == val { " selected" } else { "" };
                let escaped_id = escape_html_attr(id);
                let escaped_label = escape_html_attr(label_text);
                opts_html.push_str(&format!(
                    r#"<option value="{escaped_id}"{selected}>{escaped_label}</option>"#
                ));
            }
            let hint_html =
                build_hint_line(&get_description(schema).into_iter().collect::<Vec<_>>());
            format!(
                r#"<div class="wf-field" style="margin-top: 16px;">
  <label for="{name}" class="wf-label">{label}{req_star}</label>
  <select id="{name}" name="{name}" class="wf-select" style="width: 100%; margin-top: 4px;"{req_attr}{disabled_attr}>
    {opts_html}
  </select>
{req_msg}{hint_html}</div>
"#
            )
        }
        ("string", _) => {
            // Check for enum
            if let Some(enum_values) = schema.get("enum").and_then(|e| e.as_array()) {
                let val = value.and_then(|v| v.as_str()).unwrap_or("");
                let mut options = r#"<option value="">-- Select --</option>"#.to_string();
                for ev in enum_values {
                    let ev_str = ev.as_str().unwrap_or("");
                    let selected = if ev_str == val { " selected" } else { "" };
                    let escaped_ev = escape_html_attr(ev_str);
                    options.push_str(&format!(
                        r#"<option value="{escaped_ev}"{selected}>{escaped_ev}</option>"#
                    ));
                }
                let hint_html =
                    build_hint_line(&get_description(schema).into_iter().collect::<Vec<_>>());
                format!(
                    r#"<div class="wf-field" style="margin-top: 16px;">
  <label for="{name}" class="wf-label">{label}{req_star}</label>
  <select id="{name}" name="{name}" class="wf-select" style="width: 100%; margin-top: 4px;"{req_attr}{disabled_attr}>
    {options}
  </select>
{req_msg}{hint_html}</div>
"#
                )
            } else if schema.get("x-control").and_then(|v| v.as_str()) == Some("textarea") {
                // x-control: textarea extension renders a multi-line textarea
                let val = escape_html_attr(value.and_then(|v| v.as_str()).unwrap_or(""));
                let (constraint_attrs, hints) = string_constraints(schema, true);
                let hint_html = build_hint_line(&hints);
                format!(
                    r#"<div class="wf-field" style="margin-top: 16px;">
  <label for="{name}" class="wf-label">{label}{req_star}</label>
  <textarea id="{name}" name="{name}" rows="6" class="wf-textarea" style="width: 100%; margin-top: 4px;"{constraint_attrs}{req_attr}{readonly_attr}>{val}</textarea>
{req_msg}{hint_html}</div>
"#
                )
            } else {
                let val = escape_html_attr(value.and_then(|v| v.as_str()).unwrap_or(""));
                let (constraint_attrs, hints) = string_constraints(schema, false);
                let hint_html = build_hint_line(&hints);
                format!(
                    r#"<div class="wf-field" style="margin-top: 16px;">
  <label for="{name}" class="wf-label">{label}{req_star}</label>
  <input type="text" id="{name}" name="{name}" value="{val}" class="wf-input" style="width: 100%; margin-top: 4px;"{constraint_attrs}{req_attr}{readonly_attr}>
{req_msg}{hint_html}</div>
"#
                )
            }
        }
        ("number" | "integer", _) => {
            let raw_val = value.map(|v| v.to_string()).unwrap_or_default();
            let val = escape_html_attr(raw_val.trim_matches('"'));
            let is_integer = field_type == "integer";

            let (constraint_attrs, hints) = number_constraints(schema, is_integer);
            let hint_html = build_hint_line(&hints);

            // Default step if multipleOf not specified
            let has_step = constraint_attrs.contains("step=");
            let step = if has_step {
                String::new()
            } else if is_integer {
                r#" step="1""#.to_string()
            } else {
                r#" step="any""#.to_string()
            };

            format!(
                r#"<div class="wf-field" style="margin-top: 16px;">
  <label for="{name}" class="wf-label">{label}{req_star}</label>
  <input type="number" id="{name}" name="{name}" value="{val}"{step}{constraint_attrs} class="wf-input" style="width: 100%; margin-top: 4px;"{req_attr}{readonly_attr}>
{req_msg}{hint_html}</div>
"#
            )
        }
        ("boolean", _) => {
            let checked = value.and_then(|v| v.as_bool()).unwrap_or(false);
            let checked_attr = if checked { " checked" } else { "" };
            let hint_html =
                build_hint_line(&get_description(schema).into_iter().collect::<Vec<_>>());
            format!(
                r#"<div class="wf-field" style="margin-top: 16px;">
  <label for="{name}" class="wf-check-row">
    <input type="hidden" name="{name}" value="false">
    <input type="checkbox" id="{name}" name="{name}" value="true" class="wf-check"{checked_attr}{disabled_attr}>
    <span>{label}</span>
  </label>
{hint_html}</div>
"#
            )
        }
        ("object", _) => {
            let inner = render_form_fields_inner(
                schema,
                value,
                name,
                ref_options,
                app_slug,
                depth + 1,
                array_depth,
                read_only,
            );
            format!(
                r#"<fieldset style="border-top: 1px solid var(--hairline-dim); padding-top: 16px; margin-top: 16px;">
  <legend class="wf-label" style="padding: 0 4px;">{label}</legend>
  {inner}
</fieldset>
"#
            )
        }
        ("array", _) => {
            let items_schema = schema
                .get("items")
                .cloned()
                .unwrap_or(Value::Object(Default::default()));
            let existing_items = value.and_then(|v| v.as_array());
            let mut items_html = String::new();
            let items_is_object = items_schema.get("properties").is_some();

            if let Some(items) = existing_items {
                for (i, item) in items.iter().enumerate() {
                    let item_name = format!("{name}[{i}]");
                    let preview = array_item_preview(item, &items_schema);
                    let preview_html = if preview.is_empty() {
                        String::new()
                    } else {
                        format!(
                            r#"<span style="color: var(--fg-muted); font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 320px;">{}</span>"#,
                            escape_html_attr(&preview)
                        )
                    };
                    let item_html = if items_is_object {
                        render_form_fields_inner(
                            &items_schema,
                            Some(item),
                            &item_name,
                            ref_options,
                            app_slug,
                            depth + 1,
                            array_depth,
                            read_only,
                        )
                    } else {
                        let item_type = items_schema
                            .get("type")
                            .and_then(|t| t.as_str())
                            .unwrap_or("string");
                        let item_format = items_schema.get("format").and_then(|f| f.as_str());
                        let item_label = items_schema
                            .get("title")
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        render_field(
                            &item_name,
                            item_label,
                            item_type,
                            item_format,
                            &items_schema,
                            Some(item),
                            false,
                            ref_options,
                            app_slug,
                            depth + 1,
                            array_depth,
                            read_only,
                        )
                    };
                    let remove_button = if read_only {
                        String::new()
                    } else {
                        r#"<button type="button" onclick="this.closest('.array-item').remove()" style="color: var(--err); background: none; border: none; cursor: pointer; flex-shrink: 0;">Remove</button>"#.to_string()
                    };
                    items_html.push_str(&format!(
                        r#"<div class="array-item wf-framed" data-index="{i}">
  <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 4px;">
    {preview_html}
    {remove_button}
  </div>
  {item_html}
</div>"#,
                    ));
                }
            }

            let placeholder = format!("__IDX_{array_depth}__");
            let template_name = format!("{name}[{placeholder}]");
            let template_html = if items_is_object {
                render_form_fields_inner(
                    &items_schema,
                    None,
                    &template_name,
                    ref_options,
                    app_slug,
                    depth + 1,
                    array_depth + 1,
                    read_only,
                )
            } else {
                let item_type = items_schema
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("string");
                let item_format = items_schema.get("format").and_then(|f| f.as_str());
                let item_label = items_schema
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                render_field(
                    &template_name,
                    item_label,
                    item_type,
                    item_format,
                    &items_schema,
                    None,
                    false,
                    ref_options,
                    app_slug,
                    depth + 1,
                    array_depth + 1,
                    read_only,
                )
            };

            // Array constraints (hint only)
            let mut hints = Vec::new();
            let min_items = schema.get("minItems").and_then(|v| v.as_u64());
            let max_items = schema.get("maxItems").and_then(|v| v.as_u64());
            match (min_items, max_items) {
                (Some(min), Some(max)) => hints.push(format!("{min}–{max} items")),
                (Some(1), None) => hints.push("Min 1 item".to_string()),
                (Some(min), None) => hints.push(format!("Min {min} items")),
                (None, Some(max)) => hints.push(format!("Max {max} items")),
                _ => {}
            }
            if let Some(desc) = get_description(schema) {
                hints.push(desc);
            }
            let hint_html = build_hint_line(&hints);

            let template_and_button = if read_only {
                String::new()
            } else {
                format!(
                    r#"<template id="template-{name}">{template_html}</template>
  <button type="button" onclick="addArrayItem('{name}', {array_depth})" class="wf-btn ghost sm" style="margin-top: 8px;">+ Add Item</button>"#
                )
            };

            format!(
                r#"<div class="wf-field" style="margin-top: 16px;">
  <label class="wf-label">{label}</label>
  <div id="array-{name}" class="array-container">
    {items_html}
  </div>
  {template_and_button}
{hint_html}</div>
"#
            )
        }
        _ => {
            let val = escape_html_attr(value.and_then(|v| v.as_str()).unwrap_or(""));
            format!(
                r#"<div class="wf-field" style="margin-top: 16px;">
  <label for="{name}" class="wf-label">{label}{req_star}</label>
  <input type="text" id="{name}" name="{name}" value="{val}" class="wf-input" style="width: 100%; margin-top: 4px;"{req_attr}{readonly_attr}>
{req_msg}</div>
"#
            )
        }
    }
}

/// Extract a short preview string from an array item for display next to the Remove button.
fn array_item_preview(item: &Value, items_schema: &Value) -> String {
    // For simple string items, show the value directly
    if let Some(s) = item.as_str() {
        let truncated = if s.len() > 60 {
            format!("{}…", &s[..s.floor_char_boundary(60)])
        } else {
            s.to_string()
        };
        return truncated;
    }
    // For upload objects (non-object item schema), show filename
    if let Some(obj) = item.as_object() {
        if let Some(Value::String(filename)) = obj.get("filename") {
            return filename.clone();
        }
    }
    // For object items, show first 1-2 string property values
    if let Some(obj) = item.as_object() {
        let props = items_schema.get("properties").and_then(|p| p.as_object());
        let keys: Vec<&String> = if let Some(p) = props {
            p.keys().collect()
        } else {
            obj.keys().collect()
        };
        let mut parts = Vec::new();
        for key in keys {
            if key.starts_with('_') {
                continue;
            }
            if let Some(Value::String(s)) = obj.get(key) {
                if !s.is_empty() {
                    let truncated = if s.len() > 40 {
                        format!("{}…", &s[..s.floor_char_boundary(40)])
                    } else {
                        s.clone()
                    };
                    parts.push(truncated);
                    if parts.len() >= 2 {
                        break;
                    }
                }
            }
        }
        return parts.join(" · ");
    }
    String::new()
}

/// Parse submitted form data into a JSON Value based on the schema.
pub fn form_data_to_json(schema: &Value, form: &[(String, String)], prefix: &str) -> Value {
    form_data_to_json_inner(schema, form, prefix, 0)
}

fn form_data_to_json_inner(
    schema: &Value,
    form: &[(String, String)],
    prefix: &str,
    depth: usize,
) -> Value {
    if depth > MAX_NESTING_DEPTH {
        return Value::Object(Default::default());
    }

    let properties = match schema.get("properties").and_then(|p| p.as_object()) {
        Some(p) => p,
        None => return Value::Object(Default::default()),
    };

    let mut obj = serde_json::Map::new();

    for (key, prop_schema) in properties {
        if key == "_id" {
            continue;
        }

        let field_name = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };

        let field_type = prop_schema
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("string");
        let format = prop_schema.get("format").and_then(|f| f.as_str());

        let value = match (field_type, format) {
            ("string", Some("upload")) => {
                // Check for __current (kept existing upload)
                let current_key = format!("{field_name}.__current");
                if let Some((_, val)) = form.iter().find(|(k, _)| k == &current_key) {
                    if let Ok(parsed) = serde_json::from_str(val) {
                        parsed
                    } else {
                        Value::Null
                    }
                } else {
                    Value::Null
                }
            }
            ("string", Some("markdown-richtext")) => {
                let val = form
                    .iter()
                    .find(|(k, _)| k == &field_name)
                    .map(|(_, v)| v.as_str());
                match val {
                    Some(v) if !v.is_empty() => serde_json::from_str(v).unwrap_or(Value::Null),
                    _ => Value::Null,
                }
            }
            ("boolean", _) => {
                let val = form
                    .iter()
                    .rev()
                    .find(|(k, _)| k == &field_name)
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("false");
                Value::Bool(val == "true")
            }
            ("number", _) => {
                let val = form
                    .iter()
                    .find(|(k, _)| k == &field_name)
                    .map(|(_, v)| v.as_str());
                match val {
                    Some(v) if !v.is_empty() => v
                        .parse::<f64>()
                        .map(|n| {
                            serde_json::Number::from_f64(n)
                                .map(Value::Number)
                                .unwrap_or(Value::Null)
                        })
                        .unwrap_or(Value::Null),
                    _ => Value::Null,
                }
            }
            ("integer", _) => {
                let val = form
                    .iter()
                    .find(|(k, _)| k == &field_name)
                    .map(|(_, v)| v.as_str());
                match val {
                    Some(v) if !v.is_empty() => v
                        .parse::<i64>()
                        .map(|n| Value::Number(n.into()))
                        .unwrap_or(Value::Null),
                    _ => Value::Null,
                }
            }
            ("object", _) => form_data_to_json_inner(prop_schema, form, &field_name, depth + 1),
            ("array", _) => parse_array_form_data(prop_schema, form, &field_name, depth + 1),
            _ => {
                // String or fallback
                let val = form
                    .iter()
                    .find(|(k, _)| k == &field_name)
                    .map(|(_, v)| v.clone());
                match val {
                    Some(v) if !v.is_empty() => Value::String(v),
                    _ => Value::Null,
                }
            }
        };

        // Only include non-null values
        if !value.is_null() {
            obj.insert(key.clone(), value);
        }
    }

    Value::Object(obj)
}

fn parse_simple_form_value(
    field_type: &str,
    format: Option<&str>,
    form: &[(String, String)],
    field_name: &str,
) -> Option<Value> {
    match (field_type, format) {
        ("string", Some("upload")) => {
            let current_key = format!("{field_name}.__current");
            if let Some((_, val)) = form.iter().find(|(k, _)| k == &current_key) {
                serde_json::from_str(val).ok()
            } else {
                None
            }
        }
        ("boolean", _) => {
            let val = form
                .iter()
                .rev()
                .find(|(k, _)| k == field_name)
                .map(|(_, v)| v.as_str())
                .unwrap_or("false");
            Some(Value::Bool(val == "true"))
        }
        ("number", _) => {
            let val = form
                .iter()
                .find(|(k, _)| k == field_name)
                .map(|(_, v)| v.as_str());
            match val {
                Some(v) if !v.is_empty() => v
                    .parse::<f64>()
                    .ok()
                    .and_then(|n| serde_json::Number::from_f64(n).map(Value::Number)),
                _ => None,
            }
        }
        ("integer", _) => {
            let val = form
                .iter()
                .find(|(k, _)| k == field_name)
                .map(|(_, v)| v.as_str());
            match val {
                Some(v) if !v.is_empty() => v.parse::<i64>().ok().map(|n| Value::Number(n.into())),
                _ => None,
            }
        }
        _ => {
            let val = form
                .iter()
                .find(|(k, _)| k == field_name)
                .map(|(_, v)| v.clone());
            match val {
                Some(v) if !v.is_empty() => Some(Value::String(v)),
                _ => None,
            }
        }
    }
}

fn parse_array_form_data(
    schema: &Value,
    form: &[(String, String)],
    prefix: &str,
    depth: usize,
) -> Value {
    if depth > MAX_NESTING_DEPTH {
        return Value::Array(Vec::new());
    }

    let items_schema = match schema.get("items") {
        Some(s) => s,
        None => return Value::Array(Vec::new()),
    };

    // Find all indices used in form data
    let mut indices: Vec<usize> = Vec::new();
    let prefix_bracket = format!("{prefix}[");
    for (key, _) in form {
        if let Some(rest) = key.strip_prefix(&prefix_bracket)
            && let Some(idx_str) = rest.split(']').next()
            && let Ok(idx) = idx_str.parse::<usize>()
            && !indices.contains(&idx)
        {
            indices.push(idx);
        }
    }
    indices.sort();

    let items_is_object = items_schema.get("properties").is_some();
    let item_type = items_schema
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("string");
    let item_format = items_schema.get("format").and_then(|f| f.as_str());

    let items: Vec<Value> = indices
        .into_iter()
        .filter_map(|i| {
            let item_prefix = format!("{prefix}[{i}]");
            if items_is_object {
                Some(form_data_to_json_inner(
                    items_schema,
                    form,
                    &item_prefix,
                    depth,
                ))
            } else {
                parse_simple_form_value(item_type, item_format, form, &item_prefix)
            }
        })
        .collect();

    Value::Array(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn string_field_minlength_maxlength_renders_attrs_and_hint() {
        let schema = json!({
            "properties": {
                "name": {
                    "type": "string",
                    "title": "Name",
                    "minLength": 3,
                    "maxLength": 100
                }
            }
        });
        let html = render_form_fields(&schema, None, "", &ReferenceOptions::new(), "test-app");
        assert!(
            html.contains(r#"minlength="3""#),
            "should have minlength attr"
        );
        assert!(
            html.contains(r#"maxlength="100""#),
            "should have maxlength attr"
        );
        assert!(
            html.contains("3–100 characters"),
            "should show combined hint"
        );
    }

    #[test]
    fn string_field_pattern_renders_attr_and_hint() {
        let schema = json!({
            "properties": {
                "slug": {
                    "type": "string",
                    "title": "Slug",
                    "pattern": "^[a-z0-9-]+$"
                }
            }
        });
        let html = render_form_fields(&schema, None, "", &ReferenceOptions::new(), "test-app");
        assert!(
            html.contains(r#"pattern="^[a-z0-9-]+$""#),
            "should have pattern attr"
        );
        assert!(html.contains("Pattern:"), "should show pattern hint");
    }

    #[test]
    fn textarea_field_no_pattern_attr_but_shows_hint() {
        let schema = json!({
            "properties": {
                "bio": {
                    "type": "string",
                    "format": "textarea",
                    "title": "Bio",
                    "pattern": "^[A-Z]",
                    "maxLength": 500
                }
            }
        });
        let html = render_form_fields(&schema, None, "", &ReferenceOptions::new(), "test-app");
        assert!(
            !html.contains(r#"pattern="#),
            "textarea should not have pattern attr"
        );
        assert!(
            html.contains("Pattern:"),
            "should still show pattern as hint"
        );
        assert!(
            html.contains(r#"maxlength="500""#),
            "should have maxlength attr"
        );
    }

    #[test]
    fn field_description_renders_as_hint() {
        let schema = json!({
            "properties": {
                "email": {
                    "type": "string",
                    "title": "Email",
                    "description": "Your primary email address"
                }
            }
        });
        let html = render_form_fields(&schema, None, "", &ReferenceOptions::new(), "test-app");
        assert!(
            html.contains("Your primary email address"),
            "should show description"
        );
        assert!(
            html.contains("color: var(--fg-muted); font-size: 12px;"),
            "should use hint styling"
        );
    }

    #[test]
    fn no_constraints_no_hint_line() {
        let schema = json!({
            "properties": {
                "title": {
                    "type": "string",
                    "title": "Title"
                }
            }
        });
        let html = render_form_fields(&schema, None, "", &ReferenceOptions::new(), "test-app");
        assert!(
            !html.contains("color: var(--fg-muted); font-size: 12px;"),
            "should not have hint line"
        );
    }

    #[test]
    fn readonly_fields_disable_mutating_controls() {
        let schema = json!({
            "properties": {
                "title": {"type": "string", "title": "Title"},
                "body": {"type": "string", "format": "markdown", "title": "Body"},
                "published": {"type": "boolean", "title": "Published"},
                "status": {"type": "string", "title": "Status", "enum": ["draft", "published"]},
                "tags": {"type": "array", "title": "Tags", "items": {"type": "string"}}
            }
        });
        let data = json!({
            "title": "Hello",
            "body": "Markdown",
            "published": true,
            "status": "draft",
            "tags": ["one"]
        });
        let html = render_form_fields_readonly(
            &schema,
            Some(&data),
            "",
            &ReferenceOptions::new(),
            "test-app",
        );

        assert!(html.contains(
            r#"value="Hello" class="wf-input" style="width: 100%; margin-top: 4px;" readonly"#
        ));
        assert!(html.contains(r#"<textarea id="body" name="body" rows="12" class="wf-textarea""#));
        assert!(html.contains(r#"readonly>Markdown</textarea>"#));
        assert!(
            !html.contains("data-markdown"),
            "readonly markdown should not initialize the editor toolbar"
        );
        assert!(html.contains(r#"class="wf-check" checked disabled"#));
        assert!(
            html.contains(r#"class="wf-select" style="width: 100%; margin-top: 4px;" disabled"#)
        );
        assert!(
            !html.contains("+ Add Item"),
            "readonly arrays should not show add controls"
        );
        assert!(
            !html.contains("Remove</button>"),
            "readonly arrays should not show remove controls"
        );
    }

    #[test]
    fn upload_field_renders_drop_zone() {
        let schema = json!({
            "properties": {
                "photo": {
                    "type": "string",
                    "format": "upload",
                    "title": "Photo"
                }
            }
        });
        let html = render_form_fields(&schema, None, "", &ReferenceOptions::new(), "test-app");
        assert!(html.contains("data-upload-zone"), "should have drop zone");
        assert!(
            html.contains("data-upload-input"),
            "should have upload input"
        );
        assert!(
            html.contains("wf-dropzone-frame"),
            "should have dropzone frame"
        );
        assert!(
            html.contains("wf-dropzone-glyph"),
            "should have dropzone glyph"
        );
        assert!(
            html.contains("Drop files or click"),
            "should have prompt text"
        );
        assert!(html.contains("wf-dropzone-hint"), "should have hint text");
        assert!(
            html.contains("wf-dropzone-input"),
            "file input should use wf-dropzone-input class"
        );
    }

    #[test]
    fn upload_field_existing_image_shows_thumbnail() {
        let schema = json!({
            "properties": {
                "photo": {
                    "type": "string",
                    "format": "upload",
                    "title": "Photo"
                }
            }
        });
        let data = json!({
            "photo": {
                "hash": "abc123",
                "filename": "test.png",
                "mime": "image/png"
            }
        });
        let html = render_form_fields(
            &schema,
            Some(&data),
            "",
            &ReferenceOptions::new(),
            "test-app",
        );
        assert!(
            html.contains("<img"),
            "should show image thumbnail for image MIME"
        );
        assert!(
            html.contains("/apps/test-app/uploads/file/abc123/test.png"),
            "should link to upload"
        );
        assert!(
            html.contains("__current"),
            "should preserve hidden current field"
        );
    }

    #[test]
    fn upload_field_existing_non_image_no_thumbnail() {
        let schema = json!({
            "properties": {
                "doc": {
                    "type": "string",
                    "format": "upload",
                    "title": "Document"
                }
            }
        });
        let data = json!({
            "doc": {
                "hash": "def456",
                "filename": "readme.pdf",
                "mime": "application/pdf"
            }
        });
        let html = render_form_fields(
            &schema,
            Some(&data),
            "",
            &ReferenceOptions::new(),
            "test-app",
        );
        assert!(
            !html.contains("<img"),
            "should NOT show thumbnail for non-image"
        );
        assert!(
            html.contains("/apps/test-app/uploads/file/def456/readme.pdf"),
            "should link to upload"
        );
        assert!(
            html.contains("__current"),
            "should preserve hidden current field"
        );
    }

    #[test]
    fn number_field_min_max_renders_attrs_and_hint() {
        let schema = json!({
            "properties": {
                "price": {
                    "type": "number",
                    "title": "Price",
                    "minimum": 0.01,
                    "maximum": 9999.99
                }
            }
        });
        let html = render_form_fields(&schema, None, "", &ReferenceOptions::new(), "test-app");
        assert!(html.contains(r#"min="0.01""#), "should have min attr");
        assert!(html.contains(r#"max="9999.99""#), "should have max attr");
        assert!(html.contains("0.01–9999.99"), "should show range hint");
    }

    #[test]
    fn integer_field_exclusive_bounds_adjusted() {
        let schema = json!({
            "properties": {
                "age": {
                    "type": "integer",
                    "title": "Age",
                    "exclusiveMinimum": 0,
                    "exclusiveMaximum": 150
                }
            }
        });
        let html = render_form_fields(&schema, None, "", &ReferenceOptions::new(), "test-app");
        assert!(
            html.contains(r#"min="1""#),
            "exclusive min 0 -> min 1 for integer"
        );
        assert!(
            html.contains(r#"max="149""#),
            "exclusive max 150 -> max 149 for integer"
        );
    }

    #[test]
    fn number_field_exclusive_bounds_hint_only() {
        let schema = json!({
            "properties": {
                "rate": {
                    "type": "number",
                    "title": "Rate",
                    "exclusiveMinimum": 0
                }
            }
        });
        let html = render_form_fields(&schema, None, "", &ReferenceOptions::new(), "test-app");
        assert!(
            !html.contains(r#"min="#),
            "no min attr for exclusive float bound"
        );
        assert!(html.contains("&gt; 0"), "should show > 0 hint");
    }

    #[test]
    fn array_field_min_max_items_renders_hint() {
        let schema = json!({
            "properties": {
                "tags": {
                    "type": "array",
                    "title": "Tags",
                    "minItems": 1,
                    "maxItems": 5,
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "title": "Name" }
                        }
                    }
                }
            }
        });
        let html = render_form_fields(&schema, None, "", &ReferenceOptions::new(), "test-app");
        assert!(
            html.contains("1–5 items"),
            "should show item count range hint"
        );
        assert!(
            html.contains("color: var(--fg-muted); font-size: 12px;"),
            "should use hint styling"
        );
    }

    #[test]
    fn array_field_min_items_singular() {
        let schema = json!({
            "properties": {
                "items": {
                    "type": "array",
                    "title": "Items",
                    "minItems": 1,
                    "items": { "type": "object", "properties": { "v": { "type": "string" } } }
                }
            }
        });
        let html = render_form_fields(&schema, None, "", &ReferenceOptions::new(), "test-app");
        assert!(html.contains("Min 1 item"), "should use singular 'item'");
        assert!(!html.contains("Min 1 items"), "should not use plural for 1");
    }

    #[test]
    fn x_control_textarea_renders_textarea() {
        let schema = json!({
            "properties": {
                "body": {
                    "type": "string",
                    "title": "Body",
                    "x-control": "textarea"
                }
            }
        });
        let html = render_form_fields(&schema, None, "", &ReferenceOptions::new(), "test-app");
        assert!(
            html.contains("<textarea"),
            "x-control: textarea should render a <textarea>"
        );
        assert!(
            !html.contains("<input type=\"text\""),
            "x-control: textarea should NOT render a text input"
        );
        assert!(html.contains("rows=\"6\""), "should have rows attribute");
    }

    #[test]
    fn x_control_textarea_with_constraints() {
        let schema = json!({
            "properties": {
                "body": {
                    "type": "string",
                    "title": "Body",
                    "x-control": "textarea",
                    "maxLength": 2000,
                    "pattern": "^[A-Z]"
                }
            }
        });
        let html = render_form_fields(&schema, None, "", &ReferenceOptions::new(), "test-app");
        assert!(html.contains("<textarea"), "should render textarea");
        assert!(
            html.contains(r#"maxlength="2000""#),
            "should have maxlength attr"
        );
        assert!(
            !html.contains(r#"pattern="#),
            "textarea should not have pattern attr (hint only)"
        );
        assert!(
            html.contains("Pattern:"),
            "should show pattern as hint text"
        );
    }

    #[test]
    fn x_control_textarea_preserves_existing_value() {
        let schema = json!({
            "properties": {
                "body": {
                    "type": "string",
                    "title": "Body",
                    "x-control": "textarea"
                }
            }
        });
        let data = json!({
            "body": "Hello world"
        });
        let html = render_form_fields(
            &schema,
            Some(&data),
            "",
            &ReferenceOptions::new(),
            "test-app",
        );
        assert!(
            html.contains(">Hello world</textarea>"),
            "should preserve existing value inside textarea"
        );
    }

    #[test]
    fn number_field_multiple_of_renders_step() {
        let schema = json!({
            "properties": {
                "quantity": {
                    "type": "integer",
                    "title": "Quantity",
                    "multipleOf": 5
                }
            }
        });
        let html = render_form_fields(&schema, None, "", &ReferenceOptions::new(), "test-app");
        assert!(
            html.contains(r#"step="5""#),
            "should override step with multipleOf"
        );
        assert!(html.contains("Step: 5"), "should show step hint");
    }

    #[test]
    fn strip_array_indices_simple() {
        assert_eq!(strip_array_indices("tags[0].ref"), "tags.ref");
    }

    #[test]
    fn strip_array_indices_multiple() {
        assert_eq!(strip_array_indices("a[0].b[1].c"), "a.b.c");
    }

    #[test]
    fn strip_array_indices_no_brackets() {
        assert_eq!(strip_array_indices("simple.path"), "simple.path");
    }

    #[test]
    fn strip_array_indices_nested_deep() {
        assert_eq!(
            strip_array_indices("sections[3].items[0].author"),
            "sections.items.author"
        );
    }

    #[test]
    fn strip_array_indices_empty() {
        assert_eq!(strip_array_indices(""), "");
    }

    #[test]
    fn strip_array_indices_only_index() {
        assert_eq!(strip_array_indices("items[0]"), "items");
    }

    #[test]
    fn render_richtext_empty() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "body": {
                    "type": "string",
                    "format": "markdown-richtext",
                    "title": "Body"
                }
            }
        });
        let html = render_form_fields(&schema, None, "", &ReferenceOptions::new(), "test-app");
        assert!(
            html.contains("data-richtext"),
            "should have data-richtext attribute"
        );
        assert!(
            html.contains("data-richtext-name=\"body\""),
            "should have field name"
        );
        assert!(
            html.contains("data-richtext-app=\"test-app\""),
            "should have app slug"
        );
        assert!(html.contains("wf-modal--lg"), "should use large modal");
        assert!(
            html.contains("data-richtext-save"),
            "should have save button"
        );
        assert!(
            html.contains("data-richtext-discard"),
            "should have discard button"
        );
        assert!(html.contains("type=\"hidden\""), "should have hidden input");
    }

    #[test]
    fn render_richtext_with_value() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "body": {
                    "type": "string",
                    "format": "markdown-richtext",
                    "title": "Body"
                }
            }
        });
        let data = serde_json::json!({
            "body": {
                "markdown": "# Hello",
                "html": "<h1>Hello</h1>"
            }
        });
        let html = render_form_fields(
            &schema,
            Some(&data),
            "",
            &ReferenceOptions::new(),
            "test-app",
        );
        assert!(html.contains("Hello"), "should show preview text");
    }
}

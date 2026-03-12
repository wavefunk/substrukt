use serde_json::Value;

/// Generate HTML form fields from a JSON Schema.
pub fn render_form_fields(schema: &Value, data: Option<&Value>, prefix: &str) -> String {
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

    for (key, prop_schema) in properties {
        // Skip internal fields
        if key == "_id" {
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
) -> String {
    let req_attr = if required { " required" } else { "" };
    let req_star = if required { " *" } else { "" };

    match (field_type, format) {
        ("string", Some("textarea")) => {
            let val = value.and_then(|v| v.as_str()).unwrap_or("");
            format!(
                r#"<div class="mb-4">
  <label for="{name}" class="block text-sm font-medium text-gray-700 mb-1">{label}{req_star}</label>
  <textarea id="{name}" name="{name}" rows="6" class="w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-blue-500"{req_attr}>{val}</textarea>
</div>
"#
            )
        }
        ("string", Some("upload")) => {
            let current = value
                .and_then(|v| v.as_object())
                .map(|obj| {
                    let filename = obj
                        .get("filename")
                        .and_then(|f| f.as_str())
                        .unwrap_or("file");
                    let hash = obj.get("hash").and_then(|h| h.as_str()).unwrap_or("");
                    format!(
                        r#"<div class="mb-2 text-sm text-gray-600">Current: <a href="/uploads/file/{hash}/{filename}" class="text-blue-600 underline" target="_blank">{filename}</a></div>
    <input type="hidden" name="{name}.__current" value='{}'>"#,
                        serde_json::to_string(&value.unwrap_or(&Value::Null)).unwrap_or_default()
                    )
                })
                .unwrap_or_default();
            format!(
                r#"<div class="mb-4">
  <label for="{name}" class="block text-sm font-medium text-gray-700 mb-1">{label}{req_star}</label>
  {current}
  <input type="file" id="{name}" name="{name}" class="w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm"{req_attr}>
</div>
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
                    options.push_str(&format!(
                        r#"<option value="{ev_str}"{selected}>{ev_str}</option>"#
                    ));
                }
                format!(
                    r#"<div class="mb-4">
  <label for="{name}" class="block text-sm font-medium text-gray-700 mb-1">{label}{req_star}</label>
  <select id="{name}" name="{name}" class="w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-blue-500"{req_attr}>
    {options}
  </select>
</div>
"#
                )
            } else {
                let val = value.and_then(|v| v.as_str()).unwrap_or("");
                format!(
                    r#"<div class="mb-4">
  <label for="{name}" class="block text-sm font-medium text-gray-700 mb-1">{label}{req_star}</label>
  <input type="text" id="{name}" name="{name}" value="{val}" class="w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-blue-500"{req_attr}>
</div>
"#
                )
            }
        }
        ("number" | "integer", _) => {
            let val = value.map(|v| v.to_string()).unwrap_or_default();
            let val = val.trim_matches('"');
            let step = if field_type == "integer" {
                r#" step="1""#
            } else {
                r#" step="any""#
            };
            format!(
                r#"<div class="mb-4">
  <label for="{name}" class="block text-sm font-medium text-gray-700 mb-1">{label}{req_star}</label>
  <input type="number" id="{name}" name="{name}" value="{val}"{step} class="w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-blue-500"{req_attr}>
</div>
"#
            )
        }
        ("boolean", _) => {
            let checked = value.and_then(|v| v.as_bool()).unwrap_or(false);
            let checked_attr = if checked { " checked" } else { "" };
            format!(
                r#"<div class="mb-4">
  <label class="flex items-center gap-2">
    <input type="hidden" name="{name}" value="false">
    <input type="checkbox" name="{name}" value="true" class="rounded border-gray-300 text-blue-600 focus:ring-blue-500"{checked_attr}>
    <span class="text-sm font-medium text-gray-700">{label}</span>
  </label>
</div>
"#
            )
        }
        ("object", _) => {
            let inner = render_form_fields(schema, value, name);
            format!(
                r#"<fieldset class="mb-4 p-4 border border-gray-200 rounded-md">
  <legend class="text-sm font-medium text-gray-700 px-2">{label}</legend>
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

            if let Some(items) = existing_items {
                for (i, item) in items.iter().enumerate() {
                    let item_name = format!("{name}[{i}]");
                    items_html.push_str(&format!(
                        r#"<div class="array-item border border-gray-100 p-3 rounded mb-2" data-index="{i}">
  <div class="flex justify-end mb-1">
    <button type="button" onclick="this.closest('.array-item').remove()" class="text-red-500 text-sm hover:text-red-700">Remove</button>
  </div>
  {}
</div>"#,
                        render_form_fields(&items_schema, Some(item), &item_name)
                    ));
                }
            }

            // Template for new items (hidden, used by JS)
            let template_name = format!("{name}[__INDEX__]");
            let template_html = render_form_fields(&items_schema, None, &template_name);

            format!(
                r#"<div class="mb-4">
  <label class="block text-sm font-medium text-gray-700 mb-1">{label}</label>
  <div id="array-{name}" class="array-container">
    {items_html}
  </div>
  <template id="template-{name}">{template_html}</template>
  <button type="button" onclick="addArrayItem('{name}')" class="mt-2 px-3 py-1 text-sm bg-gray-100 border border-gray-300 rounded hover:bg-gray-200">+ Add Item</button>
</div>
"#
            )
        }
        _ => {
            let val = value.and_then(|v| v.as_str()).unwrap_or("");
            format!(
                r#"<div class="mb-4">
  <label for="{name}" class="block text-sm font-medium text-gray-700 mb-1">{label}{req_star}</label>
  <input type="text" id="{name}" name="{name}" value="{val}" class="w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm"{req_attr}>
</div>
"#
            )
        }
    }
}

/// Parse submitted form data into a JSON Value based on the schema.
pub fn form_data_to_json(schema: &Value, form: &[(String, String)], prefix: &str) -> Value {
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
            ("object", _) => form_data_to_json(prop_schema, form, &field_name),
            ("array", _) => parse_array_form_data(prop_schema, form, &field_name),
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

fn parse_array_form_data(schema: &Value, form: &[(String, String)], prefix: &str) -> Value {
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

    let items: Vec<Value> = indices
        .into_iter()
        .map(|i| {
            let item_prefix = format!("{prefix}[{i}]");
            form_data_to_json(items_schema, form, &item_prefix)
        })
        .collect();

    Value::Array(items)
}

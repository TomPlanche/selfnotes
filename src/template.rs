//! Template rendering via simple `{{placeholder}}` substitution.

use chrono::{DateTime, Datelike, Local};

/// Values available for substitution when rendering an entry template.
pub struct Context {
    /// Timestamp used for all date/time placeholders.
    pub now: DateTime<Local>,
    /// Entry name, used by custom folders (`{{name}}`).
    pub name: Option<String>,
    /// Folder name under which `fields` are exposed, e.g. `ticket` makes a
    /// field available as `{{ticket.<field>}}`.
    pub field_prefix: Option<String>,
    /// Custom folder fields, exposed as `{{<folder>.<field>}}`.
    pub fields: Vec<(String, String)>,
}

impl Context {
    /// Build a context for the current moment.
    pub fn now() -> Self {
        Self {
            now: Local::now(),
            name: None,
            field_prefix: None,
            fields: Vec::new(),
        }
    }

    /// Attach an entry name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Attach custom folder fields, exposed as `{{<prefix>.<field>}}` where
    /// `prefix` is the folder name.
    pub fn with_fields(mut self, prefix: impl Into<String>, fields: Vec<(String, String)>) -> Self {
        self.field_prefix = Some(prefix.into());
        self.fields = fields;
        self
    }

    /// Resolve a placeholder key to its value, if known.
    fn lookup(&self, key: &str) -> Option<String> {
        if let Some(prefix) = &self.field_prefix
            && let Some(field) = key
                .strip_prefix(prefix.as_str())
                .and_then(|rest| rest.strip_prefix('.'))
        {
            return self
                .fields
                .iter()
                .find(|(name, _)| name == field)
                .map(|(_, value)| value.clone());
        }

        let value = match key {
            "date" => self.now.format("%Y-%m-%d").to_string(),
            "datetime" => self.now.format("%Y-%m-%d %H:%M").to_string(),
            "time" => self.now.format("%H:%M").to_string(),
            "year" => format!("{:04}", self.now.year()),
            "month" => format!("{:02}", self.now.month()),
            "day" => format!("{:02}", self.now.day()),
            "weekday" => self.now.format("%A").to_string(),
            "name" => self.name.clone()?,
            _ => return None,
        };
        Some(value)
    }

    /// Whether `key` resolves to a non-empty value, used by `{{?key}}` blocks.
    fn is_set(&self, key: &str) -> bool {
        self.lookup(key).is_some_and(|value| !value.is_empty())
    }
}

/// Render `template`, replacing every `{{key}}` with its value and evaluating `{{?key}}...{{/key}}` conditional blocks
/// (rendered only when `key` is set to  a non-empty value).
///
/// Unknown placeholders and unbalanced blocks are left untouched so mistakes stay visible.
pub fn render(template: &str, ctx: &Context) -> String {
    let mut output = String::with_capacity(template.len());

    render_into(template, ctx, &mut output);

    output
}

/// Render `template` into `output`, recursing into conditional block bodies.
fn render_into(template: &str, ctx: &Context, output: &mut String) {
    let mut rest = template;

    while let Some(open) = rest.find("{{") {
        output.push_str(&rest[..open]);
        let after_open = &rest[open + 2..];

        let Some(close) = after_open.find("}}") else {
            // No closing braces: emit the remainder verbatim.
            output.push_str("{{");
            rest = after_open;
            continue;
        };

        let raw = &after_open[..close];
        let tag = raw.trim();
        let after_tag = &after_open[close + 2..];

        if let Some(key) = tag.strip_prefix('?') {
            // Conditional block: render its body only when the key is set.
            let key = key.trim();

            match find_block_end(after_tag) {
                Some((body, remainder)) => {
                    if ctx.is_set(key) {
                        render_into(body, ctx, output);
                    }
                    rest = remainder;
                },
                None => {
                    // Unbalanced open: preserve the original tag verbatim.
                    push_tag(output, raw);
                    rest = after_tag;
                },
            }
        } else if tag.starts_with('/') {
            // Stray close tag with no matching open: leave it intact.
            push_tag(output, raw);
            rest = after_tag;
        } else {
            match ctx.lookup(tag) {
                Some(value) => output.push_str(&value),
                // Preserve the original, unresolved placeholder.
                None => push_tag(output, raw),
            }
            rest = after_tag;
        }
    }

    output.push_str(rest);
}

/// Emit `{{raw}}` verbatim, restoring the braces stripped during scanning.
fn push_tag(output: &mut String, raw: &str) {
    output.push_str("{{");
    output.push_str(raw);
    output.push_str("}}");
}

/// Given the text right after a `{{?...}}` open tag, return the block body and the text
/// following the matching `{{/...}}`, accounting for nested blocks.
///
/// Returns `None` when no balanced close tag exists.
fn find_block_end(text: &str) -> Option<(&str, &str)> {
    let mut depth = 1usize;
    let mut cursor = 0usize;

    loop {
        let open = cursor + text[cursor..].find("{{")?;
        let after = &text[open + 2..];
        let close = after.find("}}")?;
        let tag = after[..close].trim();
        let after_close = open + 2 + close + 2;

        if tag.starts_with('?') {
            depth += 1;
        } else if tag.starts_with('/') {
            depth -= 1;

            if depth == 0 {
                return Some((&text[..open], &text[after_close..]));
            }
        }

        cursor = after_close;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixed_ctx() -> Context {
        Context {
            now: Local.with_ymd_and_hms(2026, 7, 13, 9, 5, 0).unwrap(),
            name: Some("login-bug".into()),
            field_prefix: None,
            fields: Vec::new(),
        }
    }

    #[test]
    fn substitutes_date_parts() {
        let out = render("# {{date}} ({{weekday}})\nyear={{year}}", &fixed_ctx());
        assert_eq!(out, "# 2026-07-13 (Monday)\nyear=2026");
    }

    #[test]
    fn substitutes_name() {
        assert_eq!(render("# {{name}}", &fixed_ctx()), "# login-bug");
    }

    #[test]
    fn leaves_unknown_placeholder_intact() {
        assert_eq!(render("{{nope}}", &fixed_ctx()), "{{nope}}");
    }

    #[test]
    fn missing_name_is_left_intact() {
        let ctx = Context {
            now: Local::now(),
            name: None,
            field_prefix: None,
            fields: Vec::new(),
        };

        assert_eq!(render("{{name}}", &ctx), "{{name}}");
    }

    #[test]
    fn substitutes_folder_fields_under_folder_name() {
        let ctx = fixed_ctx().with_fields("ticket", vec![("priority".into(), "high".into())]);

        assert_eq!(render("Priority: {{ticket.priority}}", &ctx), "Priority: high");
    }

    #[test]
    fn unknown_folder_field_is_left_intact() {
        let ctx = fixed_ctx().with_fields("ticket", vec![("priority".into(), "high".into())]);

        assert_eq!(render("{{ticket.missing}}", &ctx), "{{ticket.missing}}");
    }

    #[test]
    fn renders_block_when_field_is_set() {
        let ctx = fixed_ctx().with_fields("ticket", vec![("priority".into(), "high".into())]);

        let out = render(
            "{{?ticket.priority}}Priority: {{ticket.priority}}{{/ticket.priority}}",
            &ctx,
        );
        assert_eq!(out, "Priority: high");
    }

    #[test]
    fn skips_block_when_field_is_empty() {
        let ctx = fixed_ctx().with_fields("ticket", vec![("priority".into(), String::new())]);

        let out = render(
            "A{{?ticket.priority}} Priority: {{ticket.priority}}{{/ticket.priority}}B",
            &ctx,
        );
        assert_eq!(out, "AB");
    }

    #[test]
    fn skips_block_when_field_is_missing() {
        let ctx = Context {
            now: Local.with_ymd_and_hms(2026, 7, 13, 9, 5, 0).unwrap(),
            name: None,
            field_prefix: None,
            fields: Vec::new(),
        };

        assert_eq!(render("{{?name}}Name: {{name}}{{/name}}", &ctx), "");
    }

    #[test]
    fn renders_nested_blocks() {
        let ctx = fixed_ctx().with_fields("ticket", vec![("a".into(), "x".into()), ("b".into(), String::new())]);

        // Outer `a` is set, inner `b` is empty: only the outer text survives.
        let template = "{{?ticket.a}}A{{?ticket.b}}B{{/ticket.b}}C{{/ticket.a}}";
        assert_eq!(render(template, &ctx), "AC");
    }

    #[test]
    fn unbalanced_block_is_left_intact() {
        let ctx = fixed_ctx().with_fields("ticket", vec![("priority".into(), "high".into())]);

        // No matching close tag: the open tag is preserved verbatim.
        assert_eq!(render("{{?ticket.priority}}tail", &ctx), "{{?ticket.priority}}tail");
    }
}

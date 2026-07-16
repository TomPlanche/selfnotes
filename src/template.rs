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
}

/// Render `template`, replacing every `{{key}}` with its value.
///
/// Unknown placeholders are left untouched so mistakes stay visible.
pub fn render(template: &str, ctx: &Context) -> String {
    let mut output = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(open) = rest.find("{{") {
        output.push_str(&rest[..open]);
        let after_open = &rest[open + 2..];

        match after_open.find("}}") {
            Some(close) => {
                let key = after_open[..close].trim();
                match ctx.lookup(key) {
                    Some(value) => output.push_str(&value),
                    None => {
                        // Preserve the original, unresolved placeholder.
                        output.push_str("{{");
                        output.push_str(&after_open[..close]);
                        output.push_str("}}");
                    },
                }
                rest = &after_open[close + 2..];
            },
            None => {
                // No closing braces: emit the remainder verbatim.
                output.push_str("{{");
                rest = after_open;
            },
        }
    }

    output.push_str(rest);
    output
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
}

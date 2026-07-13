//! Template rendering via simple `{{placeholder}}` substitution.

use chrono::{DateTime, Datelike, Local};

/// Values available for substitution when rendering an entry template.
pub struct Context {
    /// Timestamp used for all date/time placeholders.
    pub now: DateTime<Local>,
    /// Entry name, used by custom folders (`{{name}}`).
    pub name: Option<String>,
}

impl Context {
    /// Build a context for the current moment.
    pub fn now() -> Self {
        Self {
            now: Local::now(),
            name: None,
        }
    }

    /// Attach an entry name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Resolve a placeholder key to its value, if known.
    fn lookup(&self, key: &str) -> Option<String> {
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
        };

        assert_eq!(render("{{name}}", &ctx), "{{name}}");
    }
}

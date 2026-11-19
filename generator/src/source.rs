use crate::{Context, Error, Result};
use serde_json::Value;

pub fn ident(name: &str) -> String {
    format!("`{name}`")
}

pub fn literal(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            ch if ch.is_control() => out.push_str(&format!("\\u{{{:x}}}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[derive(Default)]
pub struct Body {
    imports: Vec<String>,
    lines: Vec<String>,
    next: usize,
}

impl Body {
    pub fn import(&mut self, parts: &[&str]) -> String {
        let path = parts.iter().map(|p| ident(p)).collect::<Vec<_>>().join(".");
        let index = self
            .imports
            .iter()
            .position(|p| p == &path)
            .unwrap_or_else(|| {
                self.imports.push(path);
                self.imports.len() - 1
            });
        format!("dbType{index}")
    }

    pub fn row_type(&mut self, name: &str) -> String {
        self.import(&["scoop", "db", "row", name])
    }

    pub fn spelling(&mut self, ctx: &mut Context<'_>, ty: &Value) -> Result<String> {
        let info = ctx.describe(ty)?;
        let shape = &info["shape"];
        if shape["kind"] == "parameter" {
            let name = shape["parameter"]["name"]
                .as_str()
                .ok_or_else(|| Error::new("missing type parameter name"))?;
            return Ok(ident(name));
        }
        let path: Vec<&str> = serde::Deserialize::deserialize(&shape["sourcePath"])
            .map_err(|_| Error::new("row generation requires a nameable column type"))?;
        let mut name = match path.as_slice() {
            [local] if shape["origin"]["source"]["cone"] != ctx.core => ident(local),
            _ => self.import(&path),
        };
        if let Some(items) = ty["arguments"].as_array() {
            let args = items
                .iter()
                .map(|t| self.spelling(ctx, t))
                .collect::<Result<Vec<_>>>()?;
            name.push_str(&format!("<{}>", args.join(", ")));
        }
        Ok(name)
    }

    pub fn local(&mut self) -> String {
        let id = self.next;
        self.next += 1;
        format!("dbLocal{id}")
    }

    pub fn line(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
    }

    pub fn save(&mut self, expr: impl AsRef<str>) -> String {
        let name = self.local();
        self.line(format!("val {name} = {}", expr.as_ref()));
        name
    }

    pub fn finish(self) -> String {
        let imports = self
            .imports
            .iter()
            .enumerate()
            .map(|(i, p)| format!("import {p} as dbType{i}"));
        let statements = imports.chain(self.lines).collect::<Vec<_>>().join("\n");
        format!("{{\n{statements}\n}}")
    }
}

pub enum Argument {
    Supplied(String),
    Defaultable(String),
}

/// Omit missing arguments at the call site so the declaration evaluates its own defaults.
pub fn construct(
    body: &mut Body,
    target: &str,
    option: &str,
    args: &[(String, Argument)],
    mut supplied: Vec<String>,
) {
    for (i, (name, arg)) in args.iter().enumerate() {
        let assign = |expr: &str| format!("{} = {expr}", ident(name));
        match arg {
            Argument::Supplied(expr) => supplied.push(assign(expr)),
            Argument::Defaultable(presence) => {
                let value = body.local();
                body.line(format!(
                    "when ({presence}) {{\ncase {option}.Some(val {value}) -> {{"
                ));
                let mut present = supplied.clone();
                present.push(assign(&value));
                construct(body, target, option, &args[i + 1..], present);
                body.line(format!("}}\ncase {option}.None -> {{"));
                construct(body, target, option, &args[i + 1..], supplied);
                body.line("}\n}");
                return;
            }
        }
    }
    body.line(format!("return {target}({})", supplied.join(", ")));
}

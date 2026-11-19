use crate::source::{Body, ident};
use crate::{Context, Error, Result};
use serde_json::{Value, json};

#[derive(Clone, Copy, PartialEq)]
pub enum Direction {
    Read,
    Write,
}

impl Direction {
    pub fn interface(self) -> &'static str {
        match self {
            Self::Read => "RowReader",
            Self::Write => "RowWriter",
        }
    }
}

pub struct Mapping<'a, 'b> {
    pub ctx: &'a mut Context<'b>,
    pub direction: Direction,
    pub row_interface: Value,
    pub column_codec: Value,
    pub dependencies: Vec<(String, Value)>,
}

fn application(declaration: &Value, ty: &Value) -> Value {
    json!({"kind":"nominalApplication", "declaration":declaration, "arguments":[ty]})
}

impl Mapping<'_, '_> {
    pub fn convert(
        &mut self,
        body: &mut Body,
        ty: &Value,
        input: &str,
        origin: &Value,
    ) -> Result<String> {
        let column_type = application(&self.column_codec, ty);
        let mut injected = None;
        for (name, dep) in &self.dependencies {
            if self.ctx.subtype(dep, &column_type)? {
                if injected.is_some() {
                    return Err(Error::at(
                        origin,
                        "multiple matching ColumnCodec constructor dependencies",
                    ));
                }
                injected = Some(format!("this.{}", ident(name)));
            }
        }
        if let Some(codec) = injected {
            return Ok(self.custom(&codec, input));
        }

        let info = self.ctx.describe(ty)?;
        let shape = &info["shape"];
        if shape["origin"]["source"]["cone"] == self.ctx.core
            && let Some([name]) = shape["sourcePath"].as_array().map(Vec::as_slice)
            && let Some(name) = name.as_str()
            && ty["declaration"] == self.ctx.export(self.ctx.core, &[name])?
            && let Some(expr) = self.builtin(body, name, ty, input, origin)?
        {
            return Ok(expr);
        }

        let companion = &info["visibleCompanion"];
        if !companion.is_null() {
            if self.ctx.subtype(companion, &column_type)? {
                let ty = body.spelling(self.ctx, ty)?;
                return Ok(self.custom(&format!("{ty}.Companion"), input));
            }
            if self
                .ctx
                .subtype(companion, &application(&self.row_interface, ty))?
            {
                let ty = body.spelling(self.ctx, ty)?;
                return Ok(match self.direction {
                    Direction::Read => format!("{input}.readRecord({ty}.Companion)"),
                    Direction::Write => {
                        let row_value = body.row_type("RowValue");
                        let rows = body.row_type("Rows");
                        format!("{row_value}.Record({rows}.encodeToRow({input}, {ty}.Companion))")
                    }
                });
            }
        }
        let name: String = serde_json::from_value(info["name"].clone())?;
        Err(Error::at(
            origin,
            format!(
                "no database column mapping for `{name}`; provide a ColumnCodec or {} companion",
                self.direction.interface()
            ),
        ))
    }

    fn custom(&self, codec: &str, value: &str) -> String {
        let method = match self.direction {
            Direction::Write => "toValue",
            Direction::Read => "fromValue",
        };
        format!("{codec}.{method}({value})")
    }

    fn builtin(
        &mut self,
        body: &mut Body,
        name: &str,
        ty: &Value,
        input: &str,
        origin: &Value,
    ) -> Result<Option<String>> {
        if matches!(name, "Array" | "Option") {
            let child = &ty["arguments"][0];
            if name == "Array"
                && child["declaration"] == self.ctx.export(self.ctx.core, &["Int8"])?
            {
                return Ok(Some(match self.direction {
                    Direction::Read => format!("{input}.readBytes()"),
                    Direction::Write => format!("{}.Bytes({input})", body.row_type("RowValue")),
                }));
            }
            let item = body.local();
            let convert = self.convert(body, child, &item, origin)?;
            let element = body.spelling(self.ctx, child)?;
            let call = match (self.direction, name) {
                (Direction::Read, "Array") => format!("{input}.readArray<{element}>"),
                (Direction::Read, _) => format!("{input}.readOptional<{element}>"),
                (Direction::Write, _) => {
                    let factory = if name == "Array" {
                        "sequence"
                    } else {
                        "nullable"
                    };
                    format!("{}.{factory}<{element}>({input})", body.row_type("Rows"))
                }
            };
            return Ok(Some(format!("{call} {{ {item} -> {convert} }}")));
        }
        let (variant, suffix) = match name {
            "Int8" | "Int16" | "Int" => ("Signed", ".toLong()"),
            "Long" => ("Signed", ""),
            "UInt8" | "UInt16" | "UInt" => ("Unsigned", ".toULong()"),
            "ULong" => ("Unsigned", ""),
            "Boolean" => ("Boolean", ""),
            "Float" => ("Float32", ""),
            "Double" => ("Float64", ""),
            "String" => ("Text", ""),
            "Char" => ("Text", ".toString()"),
            _ => return Ok(None),
        };
        Ok(Some(match self.direction {
            Direction::Read => format!("{input}.read{name}()"),
            Direction::Write => format!("{}.{variant}({input}{suffix})", body.row_type("RowValue")),
        }))
    }
}

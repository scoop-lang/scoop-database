use crate::convert::{Direction, Mapping};
use crate::model::Column;
use crate::source::{Argument, Body, construct, ident, literal};
use crate::{Context, Error, Result};
use serde_json::{Value, json};

pub fn method(ctx: &mut Context<'_>, owner: &Value) -> Result<String> {
    let (direction, row, record) = match (
        ctx.method.signature.name.as_str(),
        ctx.method.signature.parameters.as_slice(),
    ) {
        ("writeRow", [value, row]) => (Direction::Write, ident(&row.name), ident(&value.name)),
        ("readRow", [row]) => (Direction::Read, ident(&row.name), String::new()),
        _ => {
            return Err(Error::new(
                "scoop.db generates RowWriter.writeRow and RowReader.readRow",
            ));
        }
    };
    let name = direction.interface();
    let path = ["scoop", "db", "row", name];
    let mut selected = None;
    for req in &ctx.method.requirements {
        let info = ctx.describe(&req.interface)?;
        let shape = &info["shape"];
        if shape["sourcePath"] != json!(path) {
            continue;
        }
        let provider = shape["origin"]["source"]["cone"]
            .as_str()
            .ok_or_else(|| Error::new("missing row interface provider"))?;
        let id = ctx.export(provider, &path)?;
        if id == req.interface["declaration"] {
            selected = Some((
                provider.to_owned(),
                id,
                req.interface["arguments"][0].clone(),
            ));
            break;
        }
    }
    let (provider, row_interface, target) = selected
        .ok_or_else(|| Error::new(format!("expected a scoop.db.row.{name} requirement")))?;
    let info = ctx.describe(&target)?;
    let kind = &info["shape"]["nominalKind"];
    if !(kind["kind"] == "struct"
        || (kind["kind"] == "class" && kind["modality"] == "final" && kind["baseClass"].is_null()))
    {
        return Err(Error::new(
            "row generation requires a struct or a final class without a class base",
        ));
    }
    let option = ctx.export(ctx.core, &["Option"])?;
    let annotation = ctx.export(&provider, &["scoop", "db", "row", "DbColumn"])?;
    let column_codec = ctx.export(&provider, &["scoop", "db", "row", "ColumnCodec"])?;
    let dependencies = ctx.members(owner)?.dependencies();
    let members = ctx.members(&target)?;
    let columns = members.columns(direction == Direction::Read, &option, &annotation)?;
    let mut mapper = Mapping {
        ctx,
        direction,
        row_interface,
        column_codec,
        dependencies,
    };
    let mut body = Body::default();
    let mut args = Vec::new();
    let option = body.import(&["Option"]);
    for column in &columns {
        let key = literal(if column.config.name.is_empty() {
            column.name
        } else {
            &column.config.name
        });
        match direction {
            Direction::Write if !column.config.ignore => {
                let value = body.save(format!("{record}.{}", ident(column.name)));
                let omit = column.optional && !column.config.required && !column.config.emit_null;
                if omit {
                    body.line(format!(
                        "when ({value}) {{\ncase {option}.None -> {{}}\ncase {option}.Some(_) -> {{"
                    ));
                }
                let converted = mapper.convert(&mut body, column.ty, &value, column.origin)?;
                body.line(format!("{row}.put({key}, {converted})"));
                if omit {
                    body.line("}\n}");
                }
            }
            Direction::Read => {
                if let Some(arg) = read(&mut mapper, &mut body, column, &row, &key, &option)? {
                    args.push((column.name.to_owned(), arg));
                }
            }
            Direction::Write => {}
        }
    }
    if direction == Direction::Read {
        body.line(format!("{row}.finish()"));
        let target = body.spelling(mapper.ctx, &target)?;
        construct(&mut body, &target, &option, &args, Vec::new());
    }
    Ok(body.finish())
}

fn read(
    mapper: &mut Mapping<'_, '_>,
    body: &mut Body,
    column: &Column<'_>,
    row: &str,
    key: &str,
    option: &str,
) -> Result<Option<Argument>> {
    if column.config.ignore {
        return Ok((!column.default).then(|| Argument::Supplied(format!("{option}.None"))));
    }
    if column.config.required || !(column.default || column.optional) {
        let cell = body.save(format!("{row}.required({key})"));
        let value = mapper.convert(body, column.ty, &cell, column.origin)?;
        return Ok(Some(Argument::Supplied(body.save(value))));
    }
    let cell = body.local();
    let value = mapper.convert(body, column.ty, &cell, column.origin)?;
    let ty = body.spelling(mapper.ctx, column.ty)?;
    let (ty, present) = if column.default {
        (format!("{option}<{ty}>"), format!("{option}.Some({value})"))
    } else {
        (ty, value)
    };
    let local = body.local();
    body.line(format!("val {local}: {ty} = when ({row}.optional({key})) {{\ncase {option}.Some(val {cell}) -> {present}\ncase {option}.None -> {option}.None\n}}"));
    Ok(Some(if column.default {
        Argument::Defaultable(local)
    } else {
        Argument::Supplied(local)
    }))
}

mod convert;
mod generate;
mod model;
mod source;

use lsp_server::{Message, Request, RequestId, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{self, BufReader, BufWriter, Stdin, Stdout};

const GENERATOR: &str = "scoop.db";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Batch {
    protocol_version: u32,
    generator: String,
    core_cone: String,
    tasks: Vec<Task>,
}

#[derive(Deserialize)]
struct Task {
    owner: Value,
    methods: Vec<Method>,
}

#[derive(Deserialize)]
struct Method {
    method: Value,
    requirements: Vec<Requirement>,
    signature: Signature,
    origin: Value,
}

#[derive(Deserialize)]
struct Requirement {
    interface: Value,
}

#[derive(Deserialize)]
struct Signature {
    name: String,
    parameters: Vec<ParameterName>,
}

#[derive(Deserialize)]
struct ParameterName {
    name: String,
}

struct Error {
    message: String,
    location: Option<Value>,
}

type Result<T> = std::result::Result<T, Error>;

impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            location: None,
        }
    }
    fn at(location: &Value, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            location: Some(location.clone()),
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::new(error.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::new(error.to_string())
    }
}

struct Session {
    input: BufReader<Stdin>,
    output: BufWriter<Stdout>,
    next_id: u64,
    exports: HashMap<(String, Vec<String>), Value>,
}

struct Context<'a> {
    session: &'a mut Session,
    method: &'a Method,
    core: &'a str,
    types: HashMap<Value, Value>,
}

impl Context<'_> {
    fn query(&mut self, query: Value) -> Result<Value> {
        let session = &mut self.session;
        session.next_id += 1;
        let id = RequestId::from(session.next_id.to_string());
        let params = json!({"context": self.method.method, "query": query});
        Message::Request(Request::new(id.clone(), "compiler/query".into(), params))
            .write(&mut session.output)?;
        match Message::read(&mut session.input)? {
            Some(Message::Response(reply)) if reply.id == id => {
                reply.response_result.map_err(|e| Error::new(e.message))
            }
            _ => Err(Error::new("expected matching compiler/query response")),
        }
    }

    fn describe(&mut self, ty: &Value) -> Result<Value> {
        if let Some(info) = self.types.get(ty) {
            return Ok(info.clone());
        }
        let reply = self.query(json!({"kind":"describeTypes","types":[ty]}))?;
        let info = reply["types"][0].clone();
        self.types.insert(ty.clone(), info.clone());
        Ok(info)
    }

    fn members(&mut self, ty: &Value) -> Result<model::Members> {
        let reply = self.query(json!({"kind":"describeMembers","types":[ty]}))?;
        Ok(serde_json::from_value(reply["members"][0].clone())?)
    }

    fn export(&mut self, provider: &str, path: &[&str]) -> Result<Value> {
        let key = (
            provider.to_owned(),
            path.iter().map(|s| s.to_string()).collect(),
        );
        if let Some(id) = self.session.exports.get(&key) {
            return Ok(id.clone());
        }
        let reply = self.query(
            json!({"kind":"lookupExport","provider":provider,"path":path,"namespace":"type"}),
        )?;
        let Some([item]) = reply["exports"].as_array().map(Vec::as_slice) else {
            return Err(Error::new(format!(
                "expected a single export for {}",
                path.join(".")
            )));
        };
        let id = item["id"].clone();
        self.session.exports.insert(key, id.clone());
        Ok(id)
    }

    fn subtype(&mut self, left: &Value, right: &Value) -> Result<bool> {
        let reply = self
            .query(json!({"kind":"typeRelation","relation":"subtype","left":left,"right":right}))?;
        serde_json::from_value(reply["holds"].clone()).map_err(Into::into)
    }
}

fn main() -> io::Result<()> {
    let mut session = Session {
        input: BufReader::new(io::stdin()),
        output: BufWriter::new(io::stdout()),
        next_id: 0,
        exports: HashMap::new(),
    };
    let Some(Message::Request(request)) = Message::read(&mut session.input)? else {
        return Err(io::Error::other("expected generator/generate request"));
    };
    if request.method != "generator/generate" {
        return Message::Response(Response::new_err(
            request.id,
            -32601,
            "expected generator/generate".into(),
        ))
        .write(&mut session.output);
    }
    let batch: Batch = match serde_json::from_value::<Batch>(request.params) {
        Ok(batch) if batch.protocol_version == 2 && batch.generator == GENERATOR => batch,
        _ => {
            return Message::Response(Response::new_err(
                request.id,
                -32602,
                "expected a protocol v2 request for scoop.db".into(),
            ))
            .write(&mut session.output);
        }
    };
    let mut bodies = Vec::new();
    let mut diagnostics = Vec::new();
    for task in &batch.tasks {
        for method in &task.methods {
            let mut ctx = Context {
                session: &mut session,
                method,
                core: &batch.core_cone,
                types: HashMap::new(),
            };
            match generate::method(&mut ctx, &task.owner) {
                Ok(source) => bodies.push(json!({"method":method.method, "source":source})),
                Err(error) => diagnostics.push(json!({
                    "method":method.method, "severity":"error", "message":error.message,
                    "location":error.location.as_ref().unwrap_or(&method.origin), "notes":[]
                })),
            }
        }
    }
    Message::Response(Response::new_ok(
        request.id,
        json!({"bodies":bodies,"diagnostics":diagnostics}),
    ))
    .write(&mut session.output)
}

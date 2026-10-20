//! JSON-RPC 2.0 as `net.minecraft.server.jsonrpc.Connection` speaks it: one object or a batch per
//! message, the error replies (`JsonRPCErrors`), and the dispatch of a request to its method.

use super::json::J;
use super::{Ctx, Fail, exec, spec};
use tracing::{debug, error, warn};

#[derive(Clone, Copy)]
enum Code {
    ParseError,
    InvalidRequest,
    MethodNotFound,
    InvalidParams,
    InternalError,
}

impl Code {
    fn parts(self) -> (i64, &'static str) {
        match self {
            Code::ParseError => (-32700, "Parse error"),
            Code::InvalidRequest => (-32600, "Invalid Request"),
            Code::MethodNotFound => (-32601, "Method not found"),
            Code::InvalidParams => (-32602, "Invalid params"),
            Code::InternalError => (-32603, "Internal error"),
        }
    }
}

/// `JsonRPCUtils.createError`: `data` is left out when it is missing or blank.
fn error_reply(id: J, code: Code, data: Option<&str>) -> J {
    let (n, message) = code.parts();
    let mut err = vec![("code", J::int(n)), ("message", J::str(message))];
    if let Some(d) = data.filter(|d| !d.trim().is_empty()) {
        err.push(("data", J::str(d)));
    }
    J::obj(vec![("jsonrpc", J::str("2.0")), ("id", id), ("error", J::obj(err))])
}

/// The reply to a text that is no JSON.
pub fn parse_error(message: &str) -> J {
    error_reply(J::Null, Code::ParseError, Some(message))
}

/// What `Connection.channelRead0` makes of a message: a reply, or none; `Err` closes the connection (a batch with an element that is no object).
pub fn handle_message(ctx: &Ctx, client: u32, message: &J) -> Result<Option<J>, ()> {
    match message {
        J::Obj(_) => Ok(handle_object(ctx, client, message)),
        J::Arr(items) => {
            let mut replies = Vec::new();
            for item in items {
                if !item.is_object() {
                    // (`getAsJsonObject` throws; the exception is no parse error, so the connection is closed.)
                    return Err(());
                }
                replies.extend(handle_object(ctx, client, item));
            }
            Ok(Some(J::Arr(replies)))
        }
        _ => Ok(Some(error_reply(J::Null, Code::InvalidRequest, None))),
    }
}

fn valid_request_id(id: &J) -> bool {
    matches!(id, J::Null | J::Num(_) | J::Str(_))
}

fn handle_object(ctx: &Ctx, client: u32, obj: &J) -> Option<J> {
    let id = obj.get("id");
    // `GsonHelper.getAsString(obj, "method", null)`: a primitive becomes its text, anything else throws.
    let method = match obj.get("method") {
        None => None,
        Some(m) => match m.primitive_string() {
            Some(s) => Some(s),
            None => return Some(internal_unknown_id("Error while handling rpc request")),
        },
    };
    let result = obj.get("result");
    let params = obj.get("params");
    let error = match obj.get("error") {
        None => None,
        Some(e) if e.is_object() => Some(e),
        Some(_) => return Some(internal_unknown_id("Error while handling rpc request")),
    };
    if let (Some(method), None, None) = (&method, result, error) {
        if let Some(id) = id
            && !valid_request_id(id)
        {
            return Some(error_reply(J::Null, Code::InvalidRequest, Some("Invalid request id - only String, Number and NULL supported")));
        }
        return handle_request(ctx, client, id, method, params);
    }
    if let (None, Some(result), None, Some(id)) = (&method, result, error, id) {
        // The reply to a request of ours; the server sends none.
        if id.is_number() {
            warn!("Received unknown response (id: {id}): {result}");
        } else {
            warn!("Received respose {result} with id {id} we did not request");
        }
        return None;
    }
    if let (None, None, Some(err)) = (&method, result, error) {
        error!("Received error (id: {}): {err}", id.map_or("null".to_owned(), ToString::to_string));
        return None;
    }
    Some(error_reply(id.cloned().unwrap_or(J::Null), Code::InvalidRequest, None))
}

fn internal_unknown_id(log: &str) -> J {
    error!("{log}");
    error_reply(J::Null, Code::InternalError, Some("Unknown error handling request - check server logs for stack trace"))
}

fn handle_request(ctx: &Ctx, client: u32, id: Option<&J>, method: &str, params: Option<&J>) -> Option<J> {
    let is_request = id.is_some();
    let reply = |code: Code, data: Option<&str>| is_request.then(|| error_reply(id.cloned().unwrap_or(J::Null), code, data));
    match dispatch(ctx, client, method, params) {
        Ok(result) => id.map(|id| J::obj(vec![("jsonrpc", J::str("2.0")), ("id", id.clone()), ("result", result)])),
        Err(Fail::InvalidParams(m)) => {
            debug!("Invalid parameter invocation {method}: {m}");
            reply(Code::InvalidParams, Some(&m))
        }
        Err(Fail::Encode(m)) => {
            error!("Failed to encode json rpc response {m}");
            reply(Code::InternalError, Some(&m))
        }
        Err(Fail::InvalidRequest(m)) => reply(Code::InvalidRequest, Some(&m)),
        Err(Fail::MethodNotFound(m)) => reply(Code::MethodNotFound, Some(&m)),
        Err(Fail::Internal(m)) => {
            error!("Error while dispatching rpc method {method}: {m}");
            reply(Code::InternalError, None)
        }
    }
}

/// `Identifier.tryParse`: the path of a method in the `minecraft` namespace.
fn method_path(method: &str) -> Option<Option<String>> {
    let valid_ns = |s: &str| s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-'));
    let valid_path = |s: &str| s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-' | '/'));
    let (ns, path) = match method.find(':') {
        Some(i) => {
            let (ns, path) = (&method[..i], &method[i + 1..]);
            if !valid_path(path) {
                return None;
            }
            if i >= 1 {
                if !valid_ns(ns) {
                    return None;
                }
                (ns, path)
            } else {
                ("minecraft", path)
            }
        }
        None => {
            if !valid_path(method) {
                return None;
            }
            ("minecraft", method)
        }
    };
    // (Only the `minecraft` namespace has methods.)
    Some((ns == "minecraft").then(|| path.to_owned()))
}

fn dispatch(ctx: &Ctx, client: u32, method: &str, params: Option<&J>) -> Result<J, Fail> {
    let Some(found) = method_path(method) else {
        return Err(Fail::InvalidRequest(format!("Failed to parse method value: {method}")));
    };
    let name = found.filter(|p| p == "rpc.discover" || spec::find(p).is_some());
    let Some(name) = name else { return Err(Fail::MethodNotFound(format!("Method not found: {method}"))) };
    if name == "rpc.discover" {
        // Parameterless; allowed before the server runs.
        parameterless(params)?;
        return Ok(spec::discover());
    }
    if !ctx.server_set() {
        return Err(Fail::InvalidRequest(format!("Method cannot be dispatched pre server initialization: {method}")));
    }
    let def = spec::find(&name).expect("found");
    let param = match def.param {
        None => {
            parameterless(params)?;
            None
        }
        Some((pname, _)) => Some(take_param(pname, params)?),
    };
    exec::call(ctx, client, &name, param)
}

/// `IncomingRpcMethod.ParameterlessMethod.apply`: no params, or an empty array.
fn parameterless(params: Option<&J>) -> Result<(), Fail> {
    match params {
        None => Ok(()),
        Some(J::Arr(a)) if a.is_empty() => Ok(()),
        Some(_) => Err(Fail::InvalidParams("Expected no params, or an empty array".into())),
    }
}

/// `IncomingRpcMethod.Method.apply`: the one parameter, by position or by name.
fn take_param<'a>(name: &str, params: Option<&'a J>) -> Result<&'a J, Fail> {
    match params {
        Some(obj @ J::Obj(_)) => obj
            .get(name)
            .ok_or_else(|| Fail::InvalidParams(format!("Params passed by-name, but expected param [{name}] does not exist"))),
        Some(J::Arr(a)) => {
            if a.len() != 1 {
                return Err(Fail::InvalidParams("Expected exactly one element in the params array".into()));
            }
            Ok(&a[0])
        }
        _ => Err(Fail::InvalidParams("Expected params as array or named".into())),
    }
}

//! Local JSON-RPC client for the experimental QEMU guest MCP service.
//! It does not provide authentication or a security boundary.
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

const PROTOCOL: &str = "2025-11-25";
const MAX_LINE: u64 = 65_536;

fn response(reader: &mut BufReader<UnixStream>, id: u64) -> Result<Value, String> {
    let mut bytes = Vec::new();
    let n = reader
        .take(MAX_LINE + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(|e| format!("socket read failed: {e}"))?;
    if n == 0 || n as u64 > MAX_LINE || !bytes.ends_with(b"\n") {
        return Err("empty, incomplete or oversized JSON-RPC response".into());
    }
    let msg: Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("invalid response JSON: {e}"))?;
    if msg.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || msg.get("id").and_then(Value::as_u64) != Some(id)
    {
        return Err("JSON-RPC version or response ID mismatch".into());
    }
    if let Some(error) = msg.get("error") {
        return Err(format!("server rejected request {id}: {error}"));
    }
    if !msg.get("result").is_some_and(Value::is_object) {
        return Err("missing or invalid JSON-RPC result".into());
    }
    Ok(msg)
}

fn send(writer: &mut UnixStream, request: &Value) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(request).map_err(|e| format!("encode failed: {e}"))?;
    if bytes.len() >= MAX_LINE as usize {
        return Err("request exceeds line limit".into());
    }
    bytes.push(b'\n');
    writer
        .write_all(&bytes)
        .map_err(|e| format!("socket write failed: {e}"))
}

fn call(
    writer: &mut UnixStream,
    reader: &mut BufReader<UnixStream>,
    id: u64,
    method: &str,
    params: Value,
    trace: &mut Vec<Value>,
) -> Result<Value, String> {
    let request = json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params});
    if let Err(error) = send(writer, &request) {
        trace.push(json!({"request":request,"failure":error}));
        return Err(error);
    }
    match response(reader, id) {
        Ok(reply) => {
            trace.push(json!({"request":request,"response":reply}));
            Ok(reply)
        }
        Err(error) => {
            trace.push(json!({"request":request,"failure":error}));
            Err(error)
        }
    }
}

fn text_result(reply: &Value) -> Result<&str, String> {
    let result = &reply["result"];
    if result["isError"].as_bool() != Some(false) {
        return Err(format!(
            "tool returned an error or missing isError: {result}"
        ));
    }
    let content = result["content"]
        .as_array()
        .ok_or("tool content is not an array")?;
    if content.len() != 1 || content[0]["type"].as_str() != Some("text") {
        return Err("unexpected tool content".into());
    }
    content[0]["text"]
        .as_str()
        .ok_or_else(|| "missing tool text".into())
}

fn parse_state(value: &str) -> Result<((u32, u32), (u32, u32)), String> {
    fn pair(value: &str) -> Result<(u32, u32), String> {
        let (x, y) = value.split_once(',').ok_or("invalid pointer pair")?;
        Ok((
            x.parse().map_err(|_| "invalid pointer x")?,
            y.parse().map_err(|_| "invalid pointer y")?,
        ))
    }
    let value = value
        .strip_prefix("human=")
        .ok_or("missing human pointer")?;
    let (human, ai) = value.split_once("; ai=").ok_or("missing AI pointer")?;
    Ok((pair(human)?, pair(ai)?))
}

fn run(socket: PathBuf, x: u32, y: u32, trace: &mut Vec<Value>) -> Result<Value, String> {
    if !(400..=773).contains(&x) || y > 573 {
        return Err("requested coordinates are outside the demo AI workspace".into());
    }
    let metadata = fs::symlink_metadata(&socket).map_err(|e| format!("socket unavailable: {e}"))?;
    if !metadata.file_type().is_socket() {
        return Err("path is not a Unix socket".into());
    }
    let mut writer =
        UnixStream::connect(&socket).map_err(|e| format!("socket connection failed: {e}"))?;
    let timeout = Some(Duration::from_secs(5));
    writer
        .set_read_timeout(timeout)
        .map_err(|e| e.to_string())?;
    writer
        .set_write_timeout(timeout)
        .map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(writer.try_clone().map_err(|e| e.to_string())?);
    let init = call(
        &mut writer,
        &mut reader,
        1,
        "initialize",
        json!({"protocolVersion":PROTOCOL,"capabilities":{},"clientInfo":{"name":"vm-mcp-client","version":"0.1.0"}}),
        trace,
    )?;
    if init["result"]["protocolVersion"].as_str() != Some(PROTOCOL)
        || !init["result"]["capabilities"]["tools"].is_object()
    {
        return Err("server did not negotiate the expected protocol or tool capability".into());
    }
    let notification = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
    if let Err(error) = send(&mut writer, &notification) {
        trace.push(json!({"notification":notification,"failure":error}));
        return Err(error);
    }
    trace.push(json!({"notification":notification}));

    let list = call(&mut writer, &mut reader, 2, "tools/list", json!({}), trace)?;
    let tools = list["result"]["tools"]
        .as_array()
        .ok_or("tool list missing")?;
    for name in ["move_ai", "pointer_state"] {
        if !tools
            .iter()
            .any(|item| item["name"].as_str() == Some(name) && item["inputSchema"].is_object())
        {
            return Err(format!("required tool {name} is not advertised"));
        }
    }

    let before = call(
        &mut writer,
        &mut reader,
        3,
        "tools/call",
        json!({"name":"pointer_state","arguments":{}}),
        trace,
    )?;
    let before_state = text_result(&before)?.to_owned();
    parse_state(&before_state)?;
    let moved = call(
        &mut writer,
        &mut reader,
        4,
        "tools/call",
        json!({"name":"move_ai","arguments":{"x":x,"y":y}}),
        trace,
    )?;
    let expected = format!("moved AI pointer to {x},{y}");
    if text_result(&moved)? != expected {
        return Err("move response does not confirm the requested coordinates".into());
    }
    let after = call(
        &mut writer,
        &mut reader,
        5,
        "tools/call",
        json!({"name":"pointer_state","arguments":{}}),
        trace,
    )?;
    let after_state = text_result(&after)?.to_owned();
    if parse_state(&after_state)?.1 != (x, y) {
        return Err("reported AI state does not match requested coordinates".into());
    }
    Ok(
        json!({"ok":true,"transport":"local Unix socket to QEMU serial","protocol":PROTOCOL,
              "before":before_state,"after":after_state,
              "scope":"guest demo only; no authenticated capability or host UI control"}),
    )
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 4 {
        eprintln!("Usage: vm-mcp-client SOCKET X Y");
        std::process::exit(2);
    }
    let coords = args[2]
        .parse::<u32>()
        .and_then(|x| args[3].parse::<u32>().map(|y| (x, y)));
    let mut trace = Vec::new();
    let result = match coords {
        Ok((x, y)) => run(PathBuf::from(&args[1]), x, y, &mut trace),
        Err(e) => Err(format!("invalid coordinates: {e}")),
    };
    match result {
        Ok(mut report) => {
            report["trace"] = json!(trace);
            println!("{}", report);
        }
        Err(error) => {
            let _ = writeln!(
                io::stderr(),
                "{}",
                json!({"ok":false,"error":error,"trace":trace})
            );
            std::process::exit(1);
        }
    }
}

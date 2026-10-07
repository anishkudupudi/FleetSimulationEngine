use crate::{Engine, Response};
use std::error::Error;
use tiny_http::{Header, Method, Request, Response as HttpResponse, Server, StatusCode};

const INDEX_HTML: &str = include_str!("../dashboard/index.html");
const APP_CSS: &str = include_str!("../dashboard/app.css");
const APP_JS: &str = include_str!("../dashboard/app.js");

pub fn run(port: u16) -> Result<(), Box<dyn Error + Send + Sync>> {
    let address = format!("127.0.0.1:{port}");
    let server = Server::http(&address)?;
    eprintln!("dashboard running at http://{address}");

    let mut engine = Engine::new();
    for request in server.incoming_requests() {
        respond(request, &mut engine);
    }

    Ok(())
}

fn respond(mut request: Request, engine: &mut Engine) {
    let method = request.method().clone();
    let path = request.url().split('?').next().unwrap_or("/");

    let response = match (method, path) {
        (Method::Get | Method::Head, _) => match static_asset(path) {
            Some((body, content_type)) => text_response(body, content_type),
            None => HttpResponse::from_string("not found").with_status_code(StatusCode(404)),
        },
        (Method::Post, "/command") => command_response(&mut request, engine),
        _ => HttpResponse::from_string("not found").with_status_code(StatusCode(404)),
    };

    let _ = request.respond(response);
}

fn command_response(
    request: &mut Request,
    engine: &mut Engine,
) -> HttpResponse<std::io::Cursor<Vec<u8>>> {
    let mut body = String::new();
    let json = match request.as_reader().read_to_string(&mut body) {
        Ok(_) => handle_command_body(engine, &body),
        Err(err) => serde_json::to_string(&Response::error(format!(
            "failed to read request body: {err}"
        )))
        .expect("serializing a response should not fail"),
    };

    text_response(&json, "application/json")
}

pub fn handle_command_body(engine: &mut Engine, body: &str) -> String {
    let response = engine
        .handle_line(body)
        .unwrap_or_else(|| Response::error("empty command"));
    serde_json::to_string(&response).expect("serializing a response should not fail")
}

pub fn static_asset(path: &str) -> Option<(&'static str, &'static str)> {
    match path {
        "/" | "/index.html" => Some((INDEX_HTML, "text/html")),
        "/app.css" => Some((APP_CSS, "text/css")),
        "/app.js" => Some((APP_JS, "application/javascript")),
        _ => None,
    }
}

fn text_response(body: &str, content_type: &str) -> HttpResponse<std::io::Cursor<Vec<u8>>> {
    HttpResponse::from_string(body).with_header(
        Header::from_bytes("Content-Type", content_type)
            .expect("static content type header should be valid"),
    )
}

use std::{convert::Infallible, net::SocketAddr};

use hyper::{
    server::conn::AddrStream,
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server, StatusCode,
};

use semantics_core::{
    api::{ApiError, ApiResponse, Query, Reply},
    AnyError,
};

use crate::app::App;

pub async fn run_server(app: App) {
    // A `MakeService` that produces a `Service` to handle each connection.
    let make_service = make_service_fn(move |conn: &AddrStream| {
        let app = app.clone();

        let addr = conn.remote_addr();
        let service = service_fn(move |req| handler(app.clone(), addr, req));

        // Return the service to hyper.
        async move { Ok::<_, Infallible>(service) }
    });

    // Run the server like above...
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));

    tracing::info!(interface=%addr, "starting web server");

    let server = Server::bind(&addr).serve(make_service);

    if let Err(e) = server.await {
        eprintln!("server error: {}", e);
    }
}

async fn handler(
    app: App,
    _addr: SocketAddr,
    req: Request<Body>,
) -> Result<Response<Body>, Infallible> {
    let res = match req.uri().path() {
        "/api/query" => handler_api_query(&app, req).await,
        path if path.starts_with("/blob/") => {
            let blob_path = path.strip_prefix("/blob/").unwrap();
            handler_blob(&app, blob_path).await
        }
        _other => not_found(),
    };

    Ok(res)
}

fn not_found() -> Response<Body> {
    Response::builder()
        .status(hyper::http::StatusCode::NOT_FOUND)
        .body(Body::from(b"Not found".to_vec()))
        .unwrap()
}

async fn handler_blob(app: &App, blob_path: &str) -> Response<Body> {
    match app.blob().get(blob_path).await {
        Ok(Some(data)) => Response::builder()
            .status(StatusCode::OK)
            .body(data.into())
            .unwrap(),
        Ok(None) => {
            not_found()
        }
        Err(err) => Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(format!("Error: {}", err).into_bytes().into())
            .unwrap(),
    }
}

async fn handler_api_query(app: &App, req: Request<Body>) -> Response<Body> {
    let res = match api_query(app, req).await {
        Ok(repl) => ApiResponse::Ok(repl),
        Err(err) => ApiResponse::Err(ApiError {
            message: err.to_string(),
        }),
    };

    let res_json = serde_json::to_vec(&res).unwrap();

    Response::builder()
        .status(StatusCode::OK)
        .header(hyper::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(hyper::header::ACCESS_CONTROL_ALLOW_METHODS, "POST")
        .body(Body::from(res_json))
        .unwrap()
}

async fn api_query(app: &App, req: Request<Body>) -> Result<Reply, AnyError> {
    let body = hyper::body::to_bytes(req.into_body()).await?;
    let query: Query = serde_json::from_slice(&body)?;
    let reply = app.run_api_query(query).await?;
    Ok(reply)
}

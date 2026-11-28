//! HTTP exposition over plain TCP or the shared BTLS stack.

use btls::{
    pkey::PKey,
    ssl::{AlpnError, Ssl, SslAcceptor, SslMethod, SslVersion, select_next_proto},
    x509::X509,
};
use bytes::Bytes;
use http_body_util::Full;
use hyper::{Method, Request, Response, StatusCode, header, service::service_fn};
use hyper_util::{
    rt::{TokioExecutor, TokioIo, TokioTimer},
    server::conn::auto::Builder,
};
use std::{convert::Infallible, io, net::SocketAddr, pin::Pin, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{TcpListener, TcpStream},
    task::JoinSet,
};

use crate::{
    RenderError, app_config, authorize_metrics_request, collect_and_render, exposition,
    metrics_requests_total, render_text_with_push_health, runtime,
};

pub type HttpResponse = Response<Full<Bytes>>;

fn response(
    status: StatusCode,
    content_type: &'static str,
    body: impl Into<Bytes>,
) -> HttpResponse {
    let mut response = Response::new(Full::new(body.into()));
    *response.status_mut() = status;
    let _ = response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static(content_type),
    );
    response
}

pub async fn handle_request<B>(request: Request<B>, peer: SocketAddr) -> HttpResponse {
    let path = request.uri().path();
    if matches!(*request.method(), Method::GET | Method::HEAD) {
        match path {
            "/" => {
                return response(
                    StatusCode::OK,
                    "text/plain; charset=utf-8",
                    "rs-linux-exporter: /metrics",
                );
            }
            "/metrics" | "/metrics.json" => {
                let token = request
                    .headers()
                    .get(header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.strip_prefix("Bearer "));
                let (endpoint, renderer, content_type): (
                    _,
                    fn() -> Result<String, RenderError>,
                    _,
                ) = if path == "/metrics" {
                    (
                        "/metrics",
                        render_text_with_push_health,
                        "text/plain; charset=utf-8",
                    )
                } else {
                    ("/metrics.json", exposition::render_json, "application/json")
                };
                metrics_requests_total().inc();
                return match authorize_metrics_request(endpoint, Some(peer.ip()), token) {
                    Ok(()) => match collect_and_render(renderer).await {
                        Ok(body) => response(StatusCode::OK, content_type, body),
                        Err((status, message)) => {
                            response(status, "text/plain; charset=utf-8", message)
                        }
                    },
                    Err((status, message)) => {
                        response(status, "text/plain; charset=utf-8", message)
                    }
                };
            }
            _ => {}
        }
    }
    if app_config().log_404_requests {
        eprintln!(
            "404 {} {} from {}",
            request.method(),
            request.uri(),
            peer.ip()
        );
    }
    response(
        StatusCode::NOT_FOUND,
        "text/plain; charset=utf-8",
        "Not Found",
    )
}

pub fn load_tls(cert: &str, key: &str) -> Result<SslAcceptor, Box<dyn std::error::Error>> {
    let mut chain = X509::stack_from_pem(&std::fs::read(cert)?)?.into_iter();
    let leaf = chain
        .next()
        .ok_or_else(|| io::Error::other("empty certificate chain"))?;
    let key = PKey::private_key_from_pem(&std::fs::read(key)?)?;
    let mut acceptor = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls())?;
    acceptor.set_min_proto_version(Some(SslVersion::TLS1_2))?;
    acceptor.set_certificate(&leaf)?;
    for cert in chain {
        acceptor.add_extra_chain_cert(cert)?;
    }
    acceptor.set_private_key(&key)?;
    acceptor.check_private_key()?;
    acceptor.set_alpn_select_callback(|_, offered| {
        select_next_proto(b"\x02h2\x08http/1.1", offered).ok_or(AlpnError::NOACK)
    });
    Ok(acceptor.build())
}

async fn serve_connection<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    stream: S,
    peer: SocketAddr,
) {
    let service = service_fn(move |request| async move {
        Ok::<_, Infallible>(handle_request(request, peer).await)
    });
    let mut builder = Builder::new(TokioExecutor::new());
    let _ = builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(Duration::from_secs(30));
    if let Err(error) = builder
        .serve_connection(TokioIo::new(stream), service)
        .await
        && runtime::debug_enabled()
    {
        eprintln!("HTTP connection from {peer} ended: {error}");
    }
}

async fn accept_connection(stream: TcpStream, peer: SocketAddr, acceptor: Option<SslAcceptor>) {
    let Some(acceptor) = acceptor else {
        serve_connection(stream, peer).await;
        return;
    };
    let handshake = async {
        let ssl = Ssl::new(acceptor.context())?;
        let mut stream = tokio_btls::SslStream::new(ssl, stream)?;
        Pin::new(&mut stream)
            .accept()
            .await
            .map_err(io::Error::other)?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(stream)
    };
    match tokio::time::timeout(Duration::from_secs(10), handshake).await {
        Ok(Ok(stream)) => serve_connection(stream, peer).await,
        Ok(Err(error)) => {
            if runtime::debug_enabled() {
                eprintln!("TLS handshake from {peer}: {error}");
            }
        }
        Err(_) => {
            if runtime::debug_enabled() {
                eprintln!("TLS handshake from {peer} timed out");
            }
        }
    }
}

pub async fn serve(bind: SocketAddr, tls: Option<SslAcceptor>) -> io::Result<()> {
    let listener = TcpListener::bind(bind).await?;
    eprintln!(
        "Listening on {} ({})",
        listener.local_addr()?,
        if tls.is_some() { "HTTPS" } else { "HTTP" }
    );
    let mut tasks = JoinSet::new();
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = terminate.recv() => break,
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
            accepted = listener.accept() => {
                let (stream, peer) = accepted?;
                let _ = tasks.spawn(accept_connection(stream, peer, tls.clone()));
            },
        }
    }
    tasks.shutdown().await;
    Ok(())
}

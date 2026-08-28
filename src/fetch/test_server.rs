use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use touche::{Body, Response, Server, StatusCode};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct MockRequest {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) headers: HashMap<String, String>,
}

pub(super) struct MockResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl MockResponse {
    pub(super) fn ok(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body,
        }
    }

    pub(super) fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    fn into_response(self) -> Response<Body> {
        let mut response = Response::builder()
            .status(StatusCode::from_u16(self.status).unwrap())
            .header("Connection", "close");
        for (name, value) in self.headers {
            response = response.header(name, value);
        }
        response.body(self.body.into()).unwrap()
    }
}

pub(super) struct MockServer {
    certificate_der: Vec<u8>,
    authority: String,
    responses: Arc<Mutex<VecDeque<MockResponse>>>,
    requests: Arc<Mutex<VecDeque<MockRequest>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MockServer {
    pub(super) fn new() -> Self {
        let certificate = generate_simple_self_signed(["127.0.0.1".to_owned()]).unwrap();
        let certificate_der = certificate.cert.der().to_vec();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let authority = listener.local_addr().unwrap().to_string();
        listener.set_nonblocking(true).unwrap();

        let private_key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
            certificate.signing_key.serialize_der(),
        ));
        let tls_config =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(
                    vec![CertificateDer::from(certificate_der.clone())],
                    private_key,
                )
                .unwrap();
        let tls_config = Arc::new(tls_config);

        let responses = Arc::new(Mutex::new(VecDeque::<MockResponse>::new()));
        let requests = Arc::new(Mutex::new(VecDeque::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let queued_responses = Arc::clone(&responses);
        let recorded_requests = Arc::clone(&requests);
        let serving_stop = Arc::clone(&stop);
        let connections = std::iter::from_fn(move || {
            loop {
                if serving_stop.load(Ordering::Relaxed) {
                    return None;
                }
                match listener.accept() {
                    Ok((stream, _)) => {
                        let connection = ServerConnection::new(Arc::clone(&tls_config)).unwrap();
                        return Some(StreamOwned::new(connection, stream));
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(error) => panic!("failed to accept mock server connection: {}", error),
                }
            }
        });
        let thread = thread::spawn(move || {
            Server::builder()
                .from_connections(connections)
                .serve_single_thread(move |request: touche::Request<Body>| {
                    let headers = request
                        .headers()
                        .iter()
                        .map(|(name, value)| {
                            (name.as_str().to_owned(), value.to_str().unwrap().to_owned())
                        })
                        .collect();
                    recorded_requests.lock().unwrap().push_back(MockRequest {
                        method: request.method().as_str().to_owned(),
                        path: request
                            .uri()
                            .path_and_query()
                            .map_or_else(|| request.uri().path(), |path| path.as_str())
                            .to_owned(),
                        headers,
                    });
                    Ok::<_, std::convert::Infallible>(
                        queued_responses
                            .lock()
                            .unwrap()
                            .pop_front()
                            .expect("no response configured for request")
                            .into_response(),
                    )
                })
                .unwrap();
        });

        Self {
            certificate_der,
            authority,
            responses,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    pub(super) fn respond(&self, response: MockResponse) {
        self.responses.lock().unwrap().push_back(response);
    }

    pub(super) fn authority(&self) -> &str {
        &self.authority
    }

    pub(super) fn certificate_der(&self) -> &[u8] {
        &self.certificate_der
    }

    pub(super) fn read_request(&mut self) -> Option<MockRequest> {
        self.requests.lock().unwrap().pop_front()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

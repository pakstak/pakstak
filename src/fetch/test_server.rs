use rcgen::generate_simple_self_signed;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use tiny_http::{Header, Response, Server, SslConfig, StatusCode};

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

    fn into_response(self) -> Response<std::io::Cursor<Vec<u8>>> {
        let body_len = self.body.len();
        let headers = self
            .headers
            .into_iter()
            .map(|(name, value)| Header::from_bytes(name, value).unwrap())
            .collect();
        Response::new(
            StatusCode(self.status),
            headers,
            std::io::Cursor::new(self.body),
            Some(body_len),
            None,
        )
    }
}

pub(super) struct MockServer {
    certificate_der: Vec<u8>,
    authority: String,
    server: Arc<Server>,
    responses: Arc<Mutex<VecDeque<MockResponse>>>,
    requests: Arc<Mutex<VecDeque<MockRequest>>>,
    thread: Option<JoinHandle<()>>,
}

impl MockServer {
    pub(super) fn new() -> Self {
        let certificate = generate_simple_self_signed(["127.0.0.1".to_owned()]).unwrap();
        let certificate_der = certificate.cert.der().to_vec();
        let server = Arc::new(
            Server::https(
                "127.0.0.1:0",
                SslConfig {
                    certificate: certificate.cert.pem().into_bytes(),
                    private_key: certificate.signing_key.serialize_pem().into_bytes(),
                },
            )
            .unwrap(),
        );
        let authority = server.server_addr().to_string();
        let responses = Arc::new(Mutex::new(VecDeque::<MockResponse>::new()));
        let requests = Arc::new(Mutex::new(VecDeque::new()));
        let serving_server = Arc::clone(&server);
        let queued_responses = Arc::clone(&responses);
        let recorded_requests = Arc::clone(&requests);
        let thread = thread::spawn(move || {
            while let Ok(request) = serving_server.recv() {
                let headers = request
                    .headers()
                    .iter()
                    .map(|header| {
                        (
                            header.field.as_str().to_ascii_lowercase().to_string(),
                            header.value.as_str().to_owned(),
                        )
                    })
                    .collect();
                recorded_requests.lock().unwrap().push_back(MockRequest {
                    method: request.method().as_str().to_owned(),
                    path: request.url().to_owned(),
                    headers,
                });
                let response = queued_responses
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("no response configured for request");
                request.respond(response.into_response()).unwrap();
            }
        });

        Self {
            certificate_der,
            authority,
            server,
            responses,
            requests,
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
        self.server.unblock();
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

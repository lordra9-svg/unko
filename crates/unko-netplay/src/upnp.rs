//! Minimal UPnP-IGD port-mapping client for NetPlay.
//!
//! Discovery uses SSDP (`M-SEARCH` to `239.255.255.250:1900`), then the device
//! description is fetched over plain HTTP and the `WANIPConnection` or
//! `WANPPPConnection` control URL is extracted. Mappings are created with a
//! SOAP `AddPortMapping` call, and `GetExternalIPAddress` supplies the public
//! endpoint to show in the UI.
//!
//! This is deliberately small and tolerant: it speaks only `http://`, parses
//! the handful of XML elements it needs with string matching instead of a full
//! XML stack, and reports every protocol or router failure so the caller can
//! fall back to a manual public endpoint.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::time::Duration;

/// SSDP multicast group used by UPnP.
const SSDP_ADDR: &str = "239.255.255.250:1900";
/// Search target for an Internet Gateway Device.
const IGD_SEARCH: &str = "urn:schemas-upnp-org:device:InternetGatewayDevice:1";
/// Service types that expose WAN port mappings, in preference order.
const WAN_SERVICES: [&str; 2] = [
    "urn:schemas-upnp-org:service:WANIPConnection:1",
    "urn:schemas-upnp-org:service:WANPPPConnection:1",
];

/// Transport protocol for the mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// TCP.
    Tcp,
    /// UDP.
    Udp,
}

impl Protocol {
    /// UPnP spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "TCP",
            Self::Udp => "UDP",
        }
    }
}

/// One port-mapping request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingRequest {
    /// Port on the router (also the port peers connect to).
    pub external_port: u16,
    /// Port on this machine.
    pub internal_port: u16,
    /// LAN address of this machine (from the socket that reaches the router is
    /// ideal; `0.0.0.0` is rejected by most IGDs).
    pub internal_client: String,
    /// Transport protocol.
    pub protocol: Protocol,
    /// Human-readable description shown by the router.
    pub description: String,
    /// Lease in seconds; `0` is permanent.
    pub lease_seconds: u32,
}

/// Default lease for mappings created with [`MappingRequest::new`]: one hour.
///
/// The mapping is also deleted explicitly on teardown; the lease is the
/// backstop for routers that ignore the delete.
pub const DEFAULT_LEASE_SECONDS: u32 = 3600;

impl MappingRequest {
    /// Builds a request with a one-hour lease.
    ///
    /// The lease is bounded so a mapping cannot outlive the app if the router
    /// ignores the explicit delete. Set [`Self::lease_seconds`] to `0` only if
    /// a permanent mapping is really wanted.
    #[must_use]
    pub fn new(internal_client: &str, port: u16, protocol: Protocol) -> Self {
        Self {
            external_port: port,
            internal_port: port,
            internal_client: internal_client.to_string(),
            protocol,
            description: "Unko NetPlay".into(),
            lease_seconds: DEFAULT_LEASE_SECONDS,
        }
    }
}

/// A discovered IGD WAN service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgdDevice {
    /// Absolute control URL for SOAP requests.
    pub control_url: String,
    /// WAN service type used in `SOAPACTION`.
    pub service_type: String,
}

/// UPnP failure with enough context for the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpnpError {
    /// Socket, HTTP or I/O failure.
    Io(String),
    /// The response could not be parsed.
    Parse(String),
    /// The router returned a SOAP fault.
    Fault {
        /// UPnP error code.
        code: u32,
        /// Router-provided description.
        description: String,
    },
    /// No IGD was found on the network.
    NotFound,
}

impl std::fmt::Display for UpnpError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(message) => write!(formatter, "I/O error: {message}"),
            Self::Parse(message) => write!(formatter, "UPnP parse error: {message}"),
            Self::Fault { code, description } => {
                write!(
                    formatter,
                    "router refused the mapping ({code}): {description}"
                )
            }
            Self::NotFound => write!(formatter, "no UPnP Internet Gateway Device found"),
        }
    }
}

impl std::error::Error for UpnpError {}

/// Discovers an IGD on the local network.
///
/// Sends one SSDP search and tries every advertised description until a WAN
/// service is found.
///
/// # Errors
///
/// Returns [`UpnpError::NotFound`] when the timeout expires without a usable
/// device, or [`UpnpError::Io`] for socket failures.
pub fn discover(timeout: Duration) -> Result<IgdDevice, UpnpError> {
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|err| UpnpError::Io(err.to_string()))?;
    socket
        .set_read_timeout(Some(timeout))
        .map_err(|err| UpnpError::Io(err.to_string()))?;
    let request = format!(
        "M-SEARCH * HTTP/1.1\r\nHOST: {SSDP_ADDR}\r\nMAN: \"ssdp:discover\"\r\nMX: 1\r\nST: {IGD_SEARCH}\r\n\r\n"
    );
    socket
        .send_to(request.as_bytes(), SSDP_ADDR)
        .map_err(|err| UpnpError::Io(err.to_string()))?;

    let mut buffer = [0u8; 4096];
    let deadline = std::time::Instant::now() + timeout;
    let mut seen = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let _ = socket.set_read_timeout(Some(remaining));
        match socket.recv_from(&mut buffer) {
            Ok((size, _)) => {
                let text = String::from_utf8_lossy(&buffer[..size]);
                if let Some(location) = response_location(&text) {
                    if seen.contains(&location) {
                        continue;
                    }
                    seen.push(location.clone());
                    if let Ok(device) = device_from_location(&location) {
                        return Ok(device);
                    }
                }
            }
            Err(_) => break,
        }
    }
    Err(UpnpError::NotFound)
}

/// Fetches `location` and extracts the WAN control URL.
///
/// Exposed for tests and for callers that already know the description URL.
///
/// # Errors
///
/// Returns an error when the description cannot be fetched or contains no WAN
/// connection service.
pub fn device_from_location(location: &str) -> Result<IgdDevice, UpnpError> {
    let xml = http_get(location)?;
    let base = base_url(location)?;
    for service in WAN_SERVICES {
        if xml.contains(service) {
            if let Some(control) = control_url_for(&xml, service) {
                let control_url = absolute_url(&base, &control)?;
                return Ok(IgdDevice {
                    control_url,
                    service_type: service.to_string(),
                });
            }
        }
    }
    Err(UpnpError::Parse(
        "device description has no WAN connection service".into(),
    ))
}

/// Adds or replaces a port mapping.
///
/// # Errors
///
/// Returns [`UpnpError::Fault`] when the router rejects the mapping, or an I/O
/// or parse error.
pub fn add_port_mapping(device: &IgdDevice, request: &MappingRequest) -> Result<(), UpnpError> {
    let body = format!(
        "<?xml version=\"1.0\"?>\r\n\
         <s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\">\
         <s:Body><u:AddPortMapping xmlns:u=\"{}\">\
         <NewRemoteHost></NewRemoteHost>\
         <NewExternalPort>{}</NewExternalPort>\
         <NewProtocol>{}</NewProtocol>\
         <NewInternalPort>{}</NewInternalPort>\
         <NewInternalClient>{}</NewInternalClient>\
         <NewEnabled>1</NewEnabled>\
         <NewPortMappingDescription>{}</NewPortMappingDescription>\
         <NewLeaseDuration>{}</NewLeaseDuration>\
         </u:AddPortMapping></s:Body></s:Envelope>",
        device.service_type,
        request.external_port,
        request.protocol.as_str(),
        request.internal_port,
        escape(&request.internal_client),
        escape(&request.description),
        request.lease_seconds
    );
    let response = soap_call(device, "AddPortMapping", &body)?;
    check_fault(&response)
}

/// Deletes a port mapping (best effort; routers may ignore it).
///
/// # Errors
///
/// Returns a transport, parse or SOAP-fault error.
pub fn delete_port_mapping(device: &IgdDevice, request: &MappingRequest) -> Result<(), UpnpError> {
    let body = format!(
        "<?xml version=\"1.0\"?>\r\n\
         <s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\">\
         <s:Body><u:DeletePortMapping xmlns:u=\"{}\">\
         <NewRemoteHost></NewRemoteHost>\
         <NewExternalPort>{}</NewExternalPort>\
         <NewProtocol>{}</NewProtocol>\
         </u:DeletePortMapping></s:Body></s:Envelope>",
        device.service_type,
        request.external_port,
        request.protocol.as_str()
    );
    let response = soap_call(device, "DeletePortMapping", &body)?;
    check_fault(&response)
}

/// An owned port mapping that removes itself when dropped.
///
/// [`PortMapping::map`] only returns a handle after `AddPortMapping` succeeds,
/// so a failed map cannot leave a delete for a mapping the router never
/// accepted. Deletion is best-effort: [`Drop`] ignores errors, and the
/// request's bounded lease is the backstop if the router ignores the delete.
#[derive(Debug)]
pub struct PortMapping {
    device: IgdDevice,
    request: MappingRequest,
    active: bool,
}

impl PortMapping {
    /// Adds the mapping and returns a handle that owns it.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`add_port_mapping`].
    pub fn map(device: IgdDevice, request: MappingRequest) -> Result<Self, UpnpError> {
        add_port_mapping(&device, &request)?;
        Ok(Self {
            device,
            request,
            active: true,
        })
    }

    /// Deletes the mapping now and consumes the handle.
    ///
    /// Errors are ignored (best effort); the later [`Drop`] is a no-op.
    pub fn unmap(mut self) {
        self.delete();
    }

    fn delete(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;
        let _ = delete_port_mapping(&self.device, &self.request);
    }
}

impl Drop for PortMapping {
    fn drop(&mut self) {
        self.delete();
    }
}

/// Reads the router's external IPv4 address.
///
/// # Errors
///
/// Returns a transport, parse or SOAP-fault error.
pub fn external_ip(device: &IgdDevice) -> Result<String, UpnpError> {
    let body = format!(
        "<?xml version=\"1.0\"?>\r\n\
         <s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\">\
         <s:Body><u:GetExternalIPAddress xmlns:u=\"{}\"/>\
         </s:Body></s:Envelope>",
        device.service_type
    );
    let response = soap_call(device, "GetExternalIPAddress", &body)?;
    check_fault(&response)?;
    element(&response, "NewExternalIPAddress")
        .filter(|ip| !ip.is_empty() && ip != "0.0.0.0")
        .ok_or_else(|| UpnpError::Parse("response has no external IP address".into()))
}

fn soap_call(device: &IgdDevice, action: &str, body: &str) -> Result<String, UpnpError> {
    http_request(
        "POST",
        &device.control_url,
        &[
            ("Content-Type", "text/xml; charset=\"utf-8\""),
            (
                "SOAPAction",
                &format!("\"{}#{action}\"", device.service_type),
            ),
        ],
        body.as_bytes(),
    )
}

fn http_get(url: &str) -> Result<String, UpnpError> {
    http_request("GET", url, &[], &[])
}

fn http_request(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Result<String, UpnpError> {
    let (host, port, path) = split_url(url)?;
    let address = format!("{host}:{port}");
    let timeout = Duration::from_secs(5);
    let mut stream =
        TcpStream::connect(&address).map_err(|err| UpnpError::Io(format!("{address}: {err}")))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|err| UpnpError::Io(err.to_string()))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|err| UpnpError::Io(err.to_string()))?;
    let mut request =
        format!("{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    if !body.is_empty() {
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|err| UpnpError::Io(err.to_string()))?;
    stream
        .write_all(body)
        .map_err(|err| UpnpError::Io(err.to_string()))?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|err| UpnpError::Io(err.to_string()))?;
    let text = String::from_utf8_lossy(&response).into_owned();
    let (head, body) = split_http(&text)?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| UpnpError::Parse("missing HTTP status".into()))?;
    if status >= 400 && !body.contains("errorCode") {
        return Err(UpnpError::Io(format!("HTTP {status} from {url}")));
    }
    Ok(body.to_string())
}

fn check_fault(response: &str) -> Result<(), UpnpError> {
    if !response.contains("Fault") {
        return Ok(());
    }
    let code = element(response, "errorCode")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let description = element(response, "errorDescription").unwrap_or_default();
    Err(UpnpError::Fault { code, description })
}

fn split_http(text: &str) -> Result<(&str, &str), UpnpError> {
    text.split_once("\r\n\r\n")
        .ok_or_else(|| UpnpError::Parse("response has no header terminator".into()))
}

fn response_location(response: &str) -> Option<String> {
    response.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("location")
            .then(|| value.trim().to_string())
    })
}

fn control_url_for(xml: &str, service: &str) -> Option<String> {
    let service_at = xml.find(service)?;
    // Search the enclosing `<service>` block: the control URL follows the
    // service type in every IGD description seen in practice.
    let rest = &xml[service_at..];
    let end = rest.find("</service>").unwrap_or(rest.len());
    element(&rest[..end], "controlURL")
}

/// Case-insensitive `<name>value</name>` lookup (first match).
fn element(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim().to_string())
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn base_url(url: &str) -> Result<String, UpnpError> {
    let (host, port, _) = split_url(url)?;
    Ok(format!("http://{host}:{port}"))
}

fn absolute_url(base: &str, control: &str) -> Result<String, UpnpError> {
    if control.starts_with("http://") || control.starts_with("https://") {
        return Ok(control.to_string());
    }
    if control.starts_with('/') {
        return Ok(format!("{base}{control}"));
    }
    Ok(format!("{base}/{control}"))
}

fn split_url(url: &str) -> Result<(String, u16, String), UpnpError> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| UpnpError::Parse(format!("unsupported URL {url}")))?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.contains(']') => (
            host.to_string(),
            port.parse::<u16>()
                .map_err(|_| UpnpError::Parse(format!("bad port in {url}")))?,
        ),
        _ => (authority.to_string(), 80),
    };
    if host.is_empty() {
        return Err(UpnpError::Parse(format!("no host in {url}")));
    }
    Ok((host, port, format!("/{path}")))
}

/// Formats the public endpoint shown in the UI.
#[must_use]
pub fn endpoint(ip: &str, port: u16) -> String {
    format!("{ip}:{port}")
}

/// Local address used as the mapping's internal client.
///
/// Opens a UDP socket toward `peer` (a LAN address or the router) and returns
/// the local IP it would use, falling back to `127.0.0.1`.
#[must_use]
pub fn local_address_toward(peer: &str) -> String {
    let Ok(remote) = peer.parse::<SocketAddr>() else {
        return "127.0.0.1".into();
    };
    let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else {
        return "127.0.0.1".into();
    };
    if socket.connect(remote).is_err() {
        return "127.0.0.1".into();
    }
    socket
        .local_addr()
        .map(|address| address.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;
    use std::time::Instant;

    const DESCRIPTION: &str = r#"<?xml version="1.0"?>
<root xmlns="urn:schemas-upnp-org:device-1-0">
  <device><deviceType>urn:schemas-upnp-org:device:InternetGatewayDevice:1</deviceType>
    <deviceList><device><deviceType>urn:schemas-upnp-org:device:WANDevice:1</deviceType>
      <deviceList><device><deviceType>urn:schemas-upnp-org:device:WANConnectionDevice:1</deviceType>
        <serviceList><service>
          <serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType>
          <controlURL>/ctl/IPConn</controlURL>
          <eventSubURL>/evt/IPConn</eventSubURL>
        </service></serviceList>
      </device></deviceList>
    </device></deviceList>
  </device>
</root>"#;

    const SSDP: &str = "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=120\r\nLOCATION: http://192.168.1.1:5000/rootDesc.xml\r\nST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\n\r\n";

    #[test]
    fn parses_ssdp_location_and_description() {
        assert_eq!(
            response_location(SSDP).as_deref(),
            Some("http://192.168.1.1:5000/rootDesc.xml")
        );
        assert_eq!(
            control_url_for(DESCRIPTION, WAN_SERVICES[0]).as_deref(),
            Some("/ctl/IPConn")
        );
        assert!(control_url_for("<root/>", WAN_SERVICES[0]).is_none());
    }

    #[test]
    fn parses_soap_faults() {
        let fault = "<s:Envelope><s:Body><s:Fault><detail><UPnPError>\
                     <errorCode>718</errorCode>\
                     <errorDescription>ConflictInMappingEntry</errorDescription>\
                     </UPnPError></detail></s:Fault></s:Body></s:Envelope>";
        assert_eq!(
            check_fault(fault),
            Err(UpnpError::Fault {
                code: 718,
                description: "ConflictInMappingEntry".into()
            })
        );
        assert_eq!(check_fault("<ok/>"), Ok(()));
    }

    #[test]
    fn splits_urls_and_builds_endpoints() {
        assert_eq!(
            split_url("http://192.168.1.1:5000/rootDesc.xml").expect("url"),
            ("192.168.1.1".to_string(), 5000, "/rootDesc.xml".to_string())
        );
        assert_eq!(split_url("http://router/desc").expect("url").1, 80);
        assert_eq!(
            absolute_url("http://10.0.0.1:80", "/ctl").expect("absolute"),
            "http://10.0.0.1:80/ctl"
        );
        assert_eq!(endpoint("203.0.113.7", 7777), "203.0.113.7:7777");
    }

    /// Reads one HTTP request, including its declared body.
    fn read_request(stream: &mut TcpStream) -> String {
        let mut request = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let size = stream.read(&mut buffer).expect("read");
            if size == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..size]);
            let text = String::from_utf8_lossy(&request).to_string();
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let length = head
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("Content-Length: ")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if body.len() >= length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&request).into_owned()
    }

    /// Serves a description, accepts one mapping and answers the IP query.
    fn mock_igd() -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().expect("accept");
                let text = read_request(&mut stream);
                let response = if text.starts_with("GET ") {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                        DESCRIPTION.len(),
                        DESCRIPTION
                    )
                } else if text.contains("AddPortMapping") {
                    "<s:Envelope><s:Body><u:AddPortMappingResponse/></s:Body></s:Envelope>"
                        .to_string()
                } else if text.contains("GetExternalIPAddress") {
                    "<s:Envelope><s:Body><u:GetExternalIPAddressResponse>\
                     <NewExternalIPAddress>203.0.113.7</NewExternalIPAddress>\
                     </u:GetExternalIPAddressResponse></s:Body></s:Envelope>"
                        .to_string()
                } else {
                    "<s:Envelope><s:Body><s:Fault><detail><UPnPError>\
                     <errorCode>401</errorCode>\
                     <errorDescription>Invalid Action</errorDescription>\
                     </UPnPError></detail></s:Fault></s:Body></s:Envelope>"
                        .to_string()
                };
                let head = if response.starts_with("HTTP/1.1") {
                    String::new()
                } else {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\nContent-Length: {}\r\n\r\n",
                        response.len()
                    )
                };
                stream
                    .write_all(format!("{head}{response}").as_bytes())
                    .expect("write");
                requests.push(text);
            }
            requests
        });
        (format!("http://127.0.0.1:{port}/rootDesc.xml"), handle)
    }

    #[test]
    fn maps_a_port_through_a_mock_igd() {
        let (location, server) = mock_igd();
        let device = device_from_location(&location).expect("device");
        assert!(device.control_url.ends_with("/ctl/IPConn"));
        assert_eq!(device.service_type, WAN_SERVICES[0]);

        let request = MappingRequest::new("192.168.1.50", 7777, Protocol::Udp);
        add_port_mapping(&device, &request).expect("mapping");
        assert_eq!(external_ip(&device).expect("ip"), "203.0.113.7");
        assert_eq!(endpoint("203.0.113.7", 7777), "203.0.113.7:7777");

        let requests = server.join().expect("server");
        assert!(requests[0].starts_with("GET /rootDesc.xml"));
        assert!(requests[1].contains("AddPortMapping"));
        assert!(requests[1].contains("<NewExternalPort>7777</NewExternalPort>"));
        assert!(requests[1].contains("<NewProtocol>UDP</NewProtocol>"));
        assert!(requests[1].contains("<NewInternalClient>192.168.1.50</NewInternalClient>"));
        assert!(requests[2].contains("GetExternalIPAddress"));
    }

    /// Serves `expected` SOAP calls, recording each request.
    ///
    /// `add_fault` makes `AddPortMapping` return a SOAP fault. The deadline
    /// keeps a test that is missing a request from hanging forever.
    fn mock_igd_control(
        expected: usize,
        add_fault: bool,
    ) -> (IgdDevice, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let port = listener.local_addr().expect("addr").port();
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(2);
            while requests.len() < expected && Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(accepted) => accepted,
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => break,
                };
                let _ = stream.set_nonblocking(false);
                let text = read_request(&mut stream);
                let response = if text.contains("AddPortMapping") {
                    if add_fault {
                        "<s:Envelope><s:Body><s:Fault><detail><UPnPError>\
                         <errorCode>718</errorCode>\
                         <errorDescription>ConflictInMappingEntry</errorDescription>\
                         </UPnPError></detail></s:Fault></s:Body></s:Envelope>"
                            .to_string()
                    } else {
                        "<s:Envelope><s:Body><u:AddPortMappingResponse/></s:Body></s:Envelope>"
                            .to_string()
                    }
                } else if text.contains("DeletePortMapping") {
                    "<s:Envelope><s:Body><u:DeletePortMappingResponse/></s:Body></s:Envelope>"
                        .to_string()
                } else {
                    "<s:Envelope><s:Body><s:Fault><detail><UPnPError>\
                     <errorCode>401</errorCode>\
                     <errorDescription>Invalid Action</errorDescription>\
                     </UPnPError></detail></s:Fault></s:Body></s:Envelope>"
                        .to_string()
                };
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\nContent-Length: {}\r\n\r\n{}",
                            response.len(),
                            response
                        )
                        .as_bytes(),
                    )
                    .expect("write");
                requests.push(text);
            }
            requests
        });
        let device = IgdDevice {
            control_url: format!("http://127.0.0.1:{port}/ctl/IPConn"),
            service_type: WAN_SERVICES[0].to_string(),
        };
        (device, handle)
    }

    fn delete_count(requests: &[String]) -> usize {
        requests
            .iter()
            .filter(|request| request.contains("DeletePortMapping"))
            .count()
    }

    #[test]
    fn netplay_mapping_lease_is_bounded() {
        let request = MappingRequest::new("192.168.1.50", 7777, Protocol::Udp);
        assert_eq!(request.lease_seconds, 3600);
        assert_ne!(request.lease_seconds, 0);
    }

    #[test]
    fn mapping_guard_deletes_on_drop() {
        let (device, server) = mock_igd_control(2, false);
        let request = MappingRequest::new("192.168.1.50", 7777, Protocol::Udp);
        let mapping = PortMapping::map(device, request).expect("map");
        drop(mapping);

        let requests = server.join().expect("server");
        assert_eq!(delete_count(&requests), 1, "requests: {requests:?}");
    }

    #[test]
    fn explicit_unmap_deletes_once() {
        let (device, server) = mock_igd_control(2, false);
        let request = MappingRequest::new("192.168.1.50", 7778, Protocol::Udp);
        let mapping = PortMapping::map(device, request).expect("map");
        mapping.unmap();

        let requests = server.join().expect("server");
        assert_eq!(delete_count(&requests), 1, "requests: {requests:?}");
    }

    #[test]
    fn map_failure_yields_no_guard() {
        let (device, server) = mock_igd_control(1, true);
        let request = MappingRequest::new("192.168.1.50", 7779, Protocol::Udp);
        let result = PortMapping::map(device, request);

        assert_eq!(
            result.err(),
            Some(UpnpError::Fault {
                code: 718,
                description: "ConflictInMappingEntry".into()
            })
        );
        let requests = server.join().expect("server");
        assert_eq!(delete_count(&requests), 0, "requests: {requests:?}");
    }

    #[test]
    fn reports_a_soap_fault() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            read_request(&mut stream);
            let response = "HTTP/1.1 500 Internal Server Error\r\n\r\n\
                <s:Fault><detail><UPnPError><errorCode>718</errorCode>\
                <errorDescription>ConflictInMappingEntry</errorDescription>\
                </UPnPError></detail></s:Fault>";
            stream.write_all(response.as_bytes()).expect("write");
        });
        let device = IgdDevice {
            control_url: format!("http://127.0.0.1:{port}/ctl/IPConn"),
            service_type: WAN_SERVICES[0].to_string(),
        };
        let request = MappingRequest::new("192.168.1.50", 7777, Protocol::Udp);
        assert_eq!(
            add_port_mapping(&device, &request),
            Err(UpnpError::Fault {
                code: 718,
                description: "ConflictInMappingEntry".into()
            })
        );
        server.join().expect("server");
    }
}

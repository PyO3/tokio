// this is copy from https://github.com/keeperofdakeys/dns-lookup
#![allow(dead_code)]

use libc;
use std::mem;
use std::ffi::{CStr, CString, NulError};
use std::net::{SocketAddr, SocketAddrV4, SocketAddrV6, Ipv4Addr, Ipv6Addr};
use std::ptr;
use std::io;
use std::fmt;
use std::error::Error;

pub const AI_PASSIVE: libc::c_int = 0x0020;

#[derive(Debug)]
/// Address family
pub enum Family {
    /// Unspecified
    Unspec,
    /// Ipv4
    Inet,
    /// Ipv6
    Inet6,
    /// Some other
    Other(libc::c_int),
}


impl Family {
    fn from_int(int: libc::c_int) -> Self {
        match int {
            0 => Family::Unspec,
            libc::AF_INET => Family::Inet,
            libc::AF_INET6 => Family::Inet6,
            v => Family::Other(v),
        }
    }

    fn to_int(&self) -> libc::c_int {
        match *self {
            Family::Unspec => 0,
            Family::Inet => libc::AF_INET,
            Family::Inet6 => libc::AF_INET6,
            Family::Other(v) => v,
        }
    }
}


#[derive(Debug)]
/// Types of Sockets
pub enum SocketType {
    /// Sequenced, reliable, connection-based byte streams.
    Stream,
    /// Connectionless, unreliable datagrams of fixed max length.
    DGram,
    /// Raw protocol interface.
    Raw,
    /// Some other
    Other(libc::c_int),
}


impl SocketType {
    fn from_int(int: libc::c_int) -> Self {
        match int {
            libc::SOCK_STREAM => SocketType::Stream,
            libc::SOCK_DGRAM => SocketType::DGram,
            libc::SOCK_RAW => SocketType::Raw,
            v => SocketType::Other(v),
        }
    }

    fn to_int(&self) -> libc::c_int {
        match *self {
            SocketType::Stream => libc::SOCK_STREAM,
            SocketType::DGram => libc::SOCK_DGRAM,
            SocketType::Raw => libc::SOCK_RAW,
            SocketType::Other(v) => v,
        }
    }
}


#[derive(Debug)]
/// Socket Protocol
pub enum Protocol {
    /// Unspecificed.
    Unspec,
    /// Local to host (pipes and file-domain).
    Local,
    /// POSIX name for PF_LOCAL.
    Unix,
    /// IP Protocol Family.
    Inet,
    TCP,
    UDP,
    Other(libc::c_int),
}


impl Protocol {
    fn from_int(int: libc::c_int) -> Self {
        match int {
            0 => Protocol::Unspec,
            1 => Protocol::Local,
            2 => Protocol::Inet,
            6 => Protocol::TCP,
            17 => Protocol::UDP,
            v => Protocol::Other(v),
        }
    }

    fn to_int(&self) -> libc::c_int {
        match *self {
            Protocol::Unspec => 0,
            Protocol::Local => libc::PF_LOCAL,
            Protocol::Unix => libc::PF_UNIX,
            Protocol::Inet => libc::PF_INET,
            Protocol::TCP => 6,
            Protocol::UDP => 17,
            Protocol::Other(v) => v,
        }
    }
}


#[derive(Debug)]
pub struct AddrInfo {
    pub flags: libc::c_int,
    pub family: Family,
    pub socktype: SocketType,
    pub protocol: Protocol,
    pub sockaddr: SocketAddr,
    pub canonname: Option<String>,
}

impl AddrInfo {
    unsafe fn from_ptr<'a>(a: *mut libc::addrinfo, port: u16) -> Result<Self, LookupError> {
        let addrinfo = *a;

        Ok(AddrInfo {
            flags: 0,
            family: Family::from_int(addrinfo.ai_family),
            socktype: SocketType::from_int(addrinfo.ai_socktype),
            protocol: Protocol::from_int(addrinfo.ai_protocol),
            sockaddr:
                sockaddr_to_addr(
                    mem::transmute(addrinfo.ai_addr), addrinfo.ai_addrlen as usize, port)?,
            canonname: if addrinfo.ai_canonname.is_null() { None } else {
                Some(CStr::from_ptr(
                    addrinfo.ai_canonname).to_str().unwrap_or("unset").to_owned()) },
        })
    }
}


fn sockaddr_to_addr(storage: &libc::sockaddr_storage, len: usize, port: u16) -> io::Result<SocketAddr> {
    match storage.ss_family as libc::c_int {
        libc::AF_INET => {
            assert!(len as usize >= mem::size_of::<libc::sockaddr_in>());
            Ok(
                unsafe {
                    let sock = *(storage as *const _ as *const libc::sockaddr_in);
                    let ip = &*(&sock.sin_addr as *const libc::in_addr as *const Ipv4Addr);
                    SocketAddr::V4(SocketAddrV4::new(ip.clone(), port))
                }
            )
        }
        libc::AF_INET6 => {
            assert!(len as usize >= mem::size_of::<libc::sockaddr_in6>());
            Ok(
                unsafe {
                    let sock = *(storage as *const _ as *const libc::sockaddr_in6);
                    let ip = &*(&sock.sin6_addr as *const libc::in6_addr as *const Ipv6Addr);
                    SocketAddr::V6(SocketAddrV6::new(ip.clone(), port, 0, 0))
                }
            )
        }
        _ => {
            Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid argument"))
        }
    }
}


pub struct LookupAddrInfo {
    port: u16,
    orig: *mut libc::addrinfo,
    cur: *mut libc::addrinfo,
}


/// Lookup a addr info via dns, return an iterator of addr infos.
pub fn lookup_addrinfo(
    host: &str, port: u16,
    family: libc::c_int, flags: libc::c_int, socktype: SocketType) -> Result<LookupAddrInfo, LookupError> {
    let c_host = CString::new(host)?;
    let mut res = ptr::null_mut();
    let mut hints = libc::addrinfo {
        ai_flags: flags,
        ai_family: family,
        ai_socktype: socktype.to_int(),
        ai_protocol: 0,
        ai_addrlen: 0,
        ai_canonname: ptr::null_mut(),
        ai_addr: ptr::null_mut(),
        ai_next: ptr::null_mut(),
    };

    unsafe {
        match libc::getaddrinfo(c_host.as_ptr(), ptr::null(), &hints, &mut res) {
            0 => Ok(LookupAddrInfo { port: port, orig: res, cur: res }),
            _ => Err(LookupError::Generic),
        }
    }
}

impl Iterator for LookupAddrInfo {
    type Item = AddrInfo;

    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            loop {
                if self.cur.is_null() {
                    return None
                } else {
                    let ret = AddrInfo::from_ptr(self.cur, self.port);
                    self.cur = (*self.cur).ai_next as *mut libc::addrinfo;
                    if let Ok(ret) = ret {
                        return Some(ret)
                    }
                }
            }
        }
    }
}

unsafe impl Sync for LookupAddrInfo {}
unsafe impl Send for LookupAddrInfo {}

impl Drop for LookupAddrInfo {
    fn drop(&mut self) { 
        unsafe { libc::freeaddrinfo(self.orig) }
    }
}


/// Errors that can occur looking up a hostname.
pub enum LookupError {
    /// A generic IO error
    IOError(io::Error),
    /// A Null Error
    NulError(NulError),
    /// Other error
    Other(String),
    /// An unspecific error
    Generic
}

impl From<io::Error> for LookupError {
    fn from(err: io::Error) -> Self {
        LookupError::IOError(err)
    }
}

impl From<NulError> for LookupError {
    fn from(err: NulError) -> Self {
        LookupError::NulError(err)
    }
}

impl<'a> From<&'a str> for LookupError {
    fn from(err: &'a str) -> Self {
        LookupError::Other(err.to_owned())
    }
}


impl Error for LookupError {
    fn description(&self) -> &str {
        match *self {
            LookupError::IOError(_) => "IO Error",
            LookupError::Other(ref err_str) => &err_str,
            LookupError::NulError(_) => "nil pointer",
            LookupError::Generic => "generic error",
        }
    }

    fn cause(&self) -> Option<&Error> {
        match *self {
            LookupError::IOError(ref err) => Some(err),
            _ => None
        }
    }
}

impl fmt::Display for LookupError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}

impl fmt::Debug for LookupError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}

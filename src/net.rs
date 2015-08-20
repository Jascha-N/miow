use std::io;
use std::mem;
use std::sync::atomic::{AtomicUsize, Ordering, ATOMIC_USIZE_INIT};
use std::net::{TcpStream, UdpSocket, SocketAddr, TcpListener};
use std::net::{SocketAddrV4, Ipv4Addr, SocketAddrV6, Ipv6Addr};
use std::os::windows::prelude::*;

use libc::{sockaddr, sockaddr_in, sockaddr_in6};
use net2::TcpBuilder;
use winapi::*;
use ws2_32::*;

/// A type to represent a buffer in which a socket address will be stored.
///
/// This type is used with the `recv_from_overlapped` function on the
/// `UdpSocketExt` trait to provide space for the overlapped I/O operation to
/// fill in the address upon completion.
#[derive(Clone, Copy)]
pub struct SocketAddrBuf {
    buf: SOCKADDR_STORAGE,
    len: c_int,
}

/// A type to represent a buffer in which an accepted socket's address will be
/// stored.
///
/// This type is used with the `accept_overlapped` method on the
/// `TcpListenerExt` trait to provide space for the overlapped I/O operation to
/// fill in the socket addresses upon completion.
#[repr(C)]
pub struct AcceptAddrsBuf {
    // For AcceptEx we've got the restriction that the addresses passed in that
    // buffer need to be at least 16 bytes more than the maximum address length
    // for the protocol in question, so add some extra here and there
    local: SOCKADDR_STORAGE,
    _pad1: [u8; 16],
    remote: SOCKADDR_STORAGE,
    _pad2: [u8; 16],
}

/// The parsed return value of `AcceptAddrsBuf`.
pub struct AcceptAddrs<'a> {
    local: LPSOCKADDR,
    local_len: c_int,
    remote: LPSOCKADDR,
    remote_len: c_int,
    _data: &'a AcceptAddrsBuf,
}

struct WsaExtension {
    guid: GUID,
    val: AtomicUsize,
}

/// Additional methods for the `TcpStream` type in the standard library.
pub trait TcpStreamExt {
    /// Execute an overlapped read I/O operation on this TCP stream.
    ///
    /// This function will issue an overlapped I/O read (via `WSARecv`) on this
    /// socket. The provided buffer will be filled in when the operation
    /// completes and the given `WSAOVERLAPPED` instance is used to track the
    /// overlapped operation.
    ///
    /// If the operation succeeds, `Ok(true)` is returned. If the operation
    /// returns an error indicating that the I/O is currently pending,
    /// `Ok(false)` is returned. Otherwise, the error associated with the
    /// operation is returned and no overlapped operation is enqueued.
    ///
    /// The number of bytes read will be returned as part of the completion
    /// notification when the I/O finishes.
    ///
    /// # Unsafety
    ///
    /// This function is unsafe because the kernel requires that the `buf` and
    /// `overlapped` pointers are valid until the end of the I/O operation. The
    /// kernel also requires that `overlapped` is unique for this I/O operation
    /// and is not in use for any other I/O.
    ///
    /// To safely use this function callers must ensure that these two input
    /// pointers are valid until the I/O operation is completed, typically via
    /// completion ports and waiting to receive the completion notification on
    /// the port.
    unsafe fn read_overlapped(&self,
                              buf: &mut [u8],
                              overlapped: &mut WSAOVERLAPPED) -> io::Result<bool>;

    /// Execute an overlapped write I/O operation on this TCP stream.
    ///
    /// This function will issue an overlapped I/O write (via `WSASend`) on this
    /// socket. The provided buffer will be written when the operation completes
    /// and the given `WSAOVERLAPPED` instance is used to track the overlapped
    /// operation.
    ///
    /// If the operation succeeds, `Ok(true)` is returned. If the operation
    /// returns an error indicating that the I/O is currently pending,
    /// `Ok(false)` is returned. Otherwise, the error associated with the
    /// operation is returned and no overlapped operation is enqueued.
    ///
    /// The number of bytes written will be returned as part of the completion
    /// notification when the I/O finishes.
    ///
    /// # Unsafety
    ///
    /// This function is unsafe because the kernel requires that the `buf` and
    /// `overlapped` pointers are valid until the end of the I/O operation. The
    /// kernel also requires that `overlapped` is unique for this I/O operation
    /// and is not in use for any other I/O.
    ///
    /// To safely use this function callers must ensure that these two input
    /// pointers are valid until the I/O operation is completed, typically via
    /// completion ports and waiting to receive the completion notification on
    /// the port.
    unsafe fn write_overlapped(&self,
                               buf: &[u8],
                               overlapped: &mut WSAOVERLAPPED) -> io::Result<bool>;
}

/// Additional methods for the `UdpSocket` type in the standard library.
pub trait UdpSocketExt {
    /// Execute an overlapped receive I/O operation on this UDP socket.
    ///
    /// This function will issue an overlapped I/O read (via `WSARecvFrom`) on
    /// this socket. The provided buffer will be filled in when the operation
    /// completes, the source from where the data came from will be written to
    /// `addr`, and the given `WSAOVERLAPPED` instance is used to track the
    /// overlapped operation.
    ///
    /// If the operation succeeds, `Ok(true)` is returned. If the operation
    /// returns an error indicating that the I/O is currently pending,
    /// `Ok(false)` is returned. Otherwise, the error associated with the
    /// operation is returned and no overlapped operation is enqueued.
    ///
    /// The number of bytes read will be returned as part of the completion
    /// notification when the I/O finishes.
    ///
    /// # Unsafety
    ///
    /// This function is unsafe because the kernel requires that the `buf`,
    /// `addr`, and `overlapped` pointers are valid until the end of the I/O
    /// operation. The kernel also requires that `overlapped` is unique for this
    /// I/O operation and is not in use for any other I/O.
    ///
    /// To safely use this function callers must ensure that these two input
    /// pointers are valid until the I/O operation is completed, typically via
    /// completion ports and waiting to receive the completion notification on
    /// the port.
    unsafe fn recv_from_overlapped(&self,
                                   buf: &mut [u8],
                                   addr: &mut SocketAddrBuf,
                                   overlapped: &mut WSAOVERLAPPED)
                                   -> io::Result<bool>;

    /// Execute an overlapped send I/O operation on this UDP socket.
    ///
    /// This function will issue an overlapped I/O write (via `WSASendTo`) on
    /// this socket to the address specified by `addr`. The provided buffer will
    /// be written when the operation completes and the given `WSAOVERLAPPED`
    /// instance is used to track the overlapped operation.
    ///
    /// If the operation succeeds, `Ok(true)` is returned. If the operation
    /// returns an error indicating that the I/O is currently pending,
    /// `Ok(false)` is returned. Otherwise, the error associated with the
    /// operation is returned and no overlapped operation is enqueued.
    ///
    /// The number of bytes written will be returned as part of the completion
    /// notification when the I/O finishes.
    ///
    /// # Unsafety
    ///
    /// This function is unsafe because the kernel requires that the `buf` and
    /// `overlapped` pointers are valid until the end of the I/O operation. The
    /// kernel also requires that `overlapped` is unique for this I/O operation
    /// and is not in use for any other I/O.
    ///
    /// To safely use this function callers must ensure that these two input
    /// pointers are valid until the I/O operation is completed, typically via
    /// completion ports and waiting to receive the completion notification on
    /// the port.
    unsafe fn send_to_overlapped(&self,
                                 buf: &[u8],
                                 addr: &SocketAddr,
                                 overlapped: &mut WSAOVERLAPPED)
                                 -> io::Result<bool>;
}

/// Additional methods for the `TcpBuilder` type in the `net2` library.
pub trait TcpBuilderExt {
    /// Attempt to consume the internal socket in this builder by executing an
    /// overlapped connect operation.
    ///
    /// This function will issue a connect operation to the address specified on
    /// the underlying socket, flagging it as an overlapped operation which will
    /// complete asynchronously. If successful this function will return the
    /// corresponding TCP stream.
    ///
    /// This function will also return whether the connect immediately
    /// succeeded or not. If `false` is returned then the I/O operation is still
    /// pending and will complete at a later date.
    ///
    /// Note that to succeed this requires that the underlying socket has
    /// previously been bound via a call to `bind` to a local address.
    ///
    /// # Unsafety
    ///
    /// This function is unsafe because the kernel requires that the
    /// `overlapped` pointer is valid until the end of the I/O operation. The
    /// kernel also requires that `overlapped` is unique for this I/O operation
    /// and is not in use for any other I/O.
    ///
    /// To safely use this function callers must ensure that this pointer is
    /// valid until the I/O operation is completed, typically via completion
    /// ports and waiting to receive the completion notification on the port.
    unsafe fn connect_overlapped(&self, addr: &SocketAddr,
                                 overlapped: &mut WSAOVERLAPPED)
                                 -> io::Result<(TcpStream, bool)>;
}

/// Additional methods for the `TcpListener` type in the standard library.
pub trait TcpListenerExt {
    /// Perform an accept operation on this listener, accepting a connection in
    /// an overlapped fashion.
    ///
    /// This function will issue an I/O request to accept an incoming connection
    /// with the specified overlapped instance. The `socket` provided must be a
    /// configured but not bound or connected socket, and if successful this
    /// will consume the internal socket of the builder to return a TCP stream.
    ///
    /// The `addrs` buffer provided will be filled in with the local and remote
    /// addresses of the connection upon completion.
    ///
    /// If the accept succeeds immediately, `Ok(stream, true)` is returned. If
    /// the connect indicates that the I/O is currently pending, `Ok(stream,
    /// false)` is returned. Otherwise, the error associated with the operation
    /// is returned and no overlapped operation is enqueued.
    ///
    /// # Unsafety
    ///
    /// This function is unsafe because the kernel requires that the
    /// `addrs` and `overlapped` pointers are valid until the end of the I/O
    /// operation. The kernel also requires that `overlapped` is unique for this
    /// I/O operation and is not in use for any other I/O.
    ///
    /// To safely use this function callers must ensure that the pointers are
    /// valid until the I/O operation is completed, typically via completion
    /// ports and waiting to receive the completion notification on the port.
    unsafe fn accept_overlapped(&self,
                                socket: &TcpBuilder,
                                addrs: &mut AcceptAddrsBuf,
                                overlapped: &mut WSAOVERLAPPED)
                                -> io::Result<(TcpStream, bool)>;
}

#[doc(hidden)]
trait NetInt {
    fn from_be(i: Self) -> Self;
    fn to_be(&self) -> Self;
}
macro_rules! doit {
    ($($t:ident)*) => ($(impl NetInt for $t {
        fn from_be(i: Self) -> Self { <$t>::from_be(i) }
        fn to_be(&self) -> Self { <$t>::to_be(*self) }
    })*)
}
doit! { i8 i16 i32 i64 isize u8 u16 u32 u64 usize }

// fn hton<I: NetInt>(i: I) -> I { i.to_be() }
fn ntoh<I: NetInt>(i: I) -> I { I::from_be(i) }

fn last_err() -> io::Result<bool> {
    let err = unsafe { WSAGetLastError() };
    if err == WSA_IO_PENDING as i32 {
        Ok(false)
    } else {
        Err(io::Error::from_raw_os_error(err))
    }
}

fn cvt(i: c_int) -> io::Result<bool> {
    if i == SOCKET_ERROR {
        last_err()
    } else {
        Ok(true)
    }
}

fn socket_addr_to_ptrs(addr: &SocketAddr) -> (*const sockaddr, c_int) {
    match *addr {
        SocketAddr::V4(ref a) => {
            (a as *const _ as *const _, mem::size_of::<sockaddr_in>() as c_int)
        }
        SocketAddr::V6(ref a) => {
            (a as *const _ as *const _, mem::size_of::<sockaddr_in6>() as c_int)
        }
    }
}

unsafe fn ptrs_to_socket_addr(ptr: *const SOCKADDR,
                              len: c_int) -> Option<SocketAddr> {
    use libc::{sockaddr_in, sockaddr_in6, sa_family_t};
    if (len as usize) < mem::size_of::<sa_family_t>() {
        return None
    }
    match (*ptr).sa_family as i32 {
        AF_INET if len as usize >= mem::size_of::<sockaddr_in>() => {
            let b = &*(ptr as *const sockaddr_in);
            let ip = ntoh(b.sin_addr.s_addr);
            let ip = Ipv4Addr::new((ip >> 24) as u8,
                                   (ip >> 16) as u8,
                                   (ip >>  8) as u8,
                                   (ip >>  0) as u8);
            Some(SocketAddr::V4(SocketAddrV4::new(ip, ntoh(b.sin_port))))
        }
        AF_INET6 if len as usize >= mem::size_of::<sockaddr_in6>() => {
            let b = &*(ptr as *const sockaddr_in6);
            let ip = Ipv6Addr::new(ntoh(b.sin6_addr.s6_addr[0]),
                                   ntoh(b.sin6_addr.s6_addr[1]),
                                   ntoh(b.sin6_addr.s6_addr[2]),
                                   ntoh(b.sin6_addr.s6_addr[3]),
                                   ntoh(b.sin6_addr.s6_addr[4]),
                                   ntoh(b.sin6_addr.s6_addr[5]),
                                   ntoh(b.sin6_addr.s6_addr[6]),
                                   ntoh(b.sin6_addr.s6_addr[7]));
            let addr = SocketAddrV6::new(ip, ntoh(b.sin6_port),
                                         ntoh(b.sin6_flowinfo),
                                         ntoh(b.sin6_scope_id));
            Some(SocketAddr::V6(addr))
        }
        _ => None
    }
}

impl TcpStreamExt for TcpStream {
    unsafe fn read_overlapped(&self, buf: &mut [u8],
                              overlapped: &mut OVERLAPPED) -> io::Result<bool> {
        let mut buf = WSABUF {
            len: buf.len() as u_long,
            buf: buf.as_mut_ptr() as *mut _,
        };
        let mut flags = 0;
        let r = WSARecv(self.as_raw_socket(), &mut buf, 1,
                        0 as *mut _, &mut flags, overlapped, None);
        cvt(r)
    }

    unsafe fn write_overlapped(&self, buf: &[u8],
                               overlapped: &mut OVERLAPPED) -> io::Result<bool> {
        let mut buf = WSABUF {
            len: buf.len() as u_long,
            buf: buf.as_ptr() as *mut _,
        };
        let r = WSASend(self.as_raw_socket(), &mut buf, 1,
                        0 as *mut _, 0, overlapped, None);
        cvt(r)
    }
}

impl UdpSocketExt for UdpSocket {
    unsafe fn recv_from_overlapped(&self,
                                   buf: &mut [u8],
                                   addr: &mut SocketAddrBuf,
                                   overlapped: &mut WSAOVERLAPPED)
                                   -> io::Result<bool> {
        let mut buf = WSABUF {
            len: buf.len() as u_long,
            buf: buf.as_mut_ptr() as *mut _,
        };
        let mut flags = 0;
        let r = WSARecvFrom(self.as_raw_socket(), &mut buf, 1,
                            0 as *mut _, &mut flags,
                            &mut addr.buf as *mut _ as *mut _,
                            &mut addr.len,
                            overlapped, None);
        cvt(r)
    }

    unsafe fn send_to_overlapped(&self,
                                 buf: &[u8],
                                 addr: &SocketAddr,
                                 overlapped: &mut WSAOVERLAPPED)
                                 -> io::Result<bool> {
        let (addr_buf, addr_len) = socket_addr_to_ptrs(addr);
        let mut buf = WSABUF {
            len: buf.len() as u_long,
            buf: buf.as_ptr() as *mut _,
        };
        let r = WSASendTo(self.as_raw_socket(), &mut buf, 1,
                          0 as *mut _, 0,
                          addr_buf as *const _, addr_len,
                          overlapped, None);
        cvt(r)
    }
}

impl TcpBuilderExt for TcpBuilder {
    unsafe fn connect_overlapped(&self, addr: &SocketAddr,
                                 overlapped: &mut WSAOVERLAPPED)
                                 -> io::Result<(TcpStream, bool)> {
        static CONNECTEX: WsaExtension = WsaExtension {
            guid: GUID {
                Data1: 0x25a207b9,
                Data2: 0xddf3,
                Data3: 0x4660,
                Data4: [0x8e, 0xe9, 0x76, 0xe5, 0x8c, 0x74, 0x06, 0x3e],
            },
            val: ATOMIC_USIZE_INIT,
        };
        type ConnectEx = unsafe extern "system" fn(SOCKET, *const sockaddr,
                                                   c_int, PVOID, DWORD, LPDWORD,
                                                   LPOVERLAPPED) -> BOOL;

        let ptr = try!(CONNECTEX.get(self.as_raw_socket()));
        assert!(ptr != 0);
        let connect_ex = mem::transmute::<_, ConnectEx>(ptr);

        let (addr_buf, addr_len) = socket_addr_to_ptrs(addr);
        let r = connect_ex(self.as_raw_socket(), addr_buf, addr_len,
                           0 as *mut _, 0, 0 as *mut _, overlapped);
        let succeeded = if r == TRUE {
            true
        } else {
            try!(last_err())
        };
        let stream = try!(self.to_tcp_stream());
        Ok((stream, succeeded))
    }
}

impl TcpListenerExt for TcpListener {
    unsafe fn accept_overlapped(&self,
                                socket: &TcpBuilder,
                                addrs: &mut AcceptAddrsBuf,
                                overlapped: &mut WSAOVERLAPPED)
                                -> io::Result<(TcpStream, bool)> {
        static ACCEPTEX: WsaExtension = WsaExtension {
            guid: GUID {
                Data1: 0xb5367df1,
                Data2: 0xcbac,
                Data3: 0x11cf,
                Data4: [0x95, 0xca, 0x00, 0x80, 0x5f, 0x48, 0xa1, 0x92],
            },
            val: ATOMIC_USIZE_INIT,
        };
        type AcceptEx = unsafe extern "system" fn(SOCKET, SOCKET, PVOID,
                                                  DWORD, DWORD, DWORD, LPDWORD,
                                                  LPOVERLAPPED) -> BOOL;

        let ptr = try!(ACCEPTEX.get(self.as_raw_socket()));
        assert!(ptr != 0);
        let accept_ex = mem::transmute::<_, AcceptEx>(ptr);

        let mut bytes = 0;
        let (a, b, c, d) = addrs.args();
        let r = accept_ex(self.as_raw_socket(), socket.as_raw_socket(),
                          a, b, c, d, &mut bytes, overlapped);
        let succeeded = if r == TRUE {
            true
        } else {
            try!(last_err())
        };
        // NB: this unwrap() should be guaranteed to succeed, and this is an
        // assert that it does indeed succeed.
        Ok((socket.to_tcp_stream().unwrap(), succeeded))
    }
}

impl SocketAddrBuf {
    /// Creates a new blank socket address buffer.
    ///
    /// This should be used before a call to `recv_from_overlapped` overlapped
    /// to create an instance to pass down.
    pub fn new() -> SocketAddrBuf {
        SocketAddrBuf {
            buf: unsafe { mem::zeroed() },
            len: mem::size_of::<SOCKADDR_STORAGE>() as c_int,
        }
    }

    /// Parses this buffer to return a standard socket address.
    ///
    /// This function should be called after the buffer has been filled in with
    /// a call to `recv_from_overlapped` being completed. It will interpret the
    /// address filled in and return the standard socket address type.
    ///
    /// If an error is encountered then `None` is returned.
    pub fn to_socket_addr(&self) -> Option<SocketAddr> {
        unsafe {
            ptrs_to_socket_addr(&self.buf as *const _ as *const _, self.len)
        }
    }
}

static GETACCEPTEXSOCKADDRS: WsaExtension = WsaExtension {
    guid: GUID {
        Data1: 0xb5367df2,
        Data2: 0xcbac,
        Data3: 0x11cf,
        Data4: [0x95, 0xca, 0x00, 0x80, 0x5f, 0x48, 0xa1, 0x92],
    },
    val: ATOMIC_USIZE_INIT,
};
type GetAcceptExSockaddrs = unsafe extern "system" fn(PVOID, DWORD, DWORD, DWORD,
                                                      *mut LPSOCKADDR, LPINT,
                                                      *mut LPSOCKADDR, LPINT);

impl AcceptAddrsBuf {
    /// Creates a new blank buffer ready to be passed to a call to
    /// `accept_overlapped`.
    pub fn new() -> AcceptAddrsBuf {
        unsafe { mem::zeroed() }
    }

    /// Parses the data contained in this address buffer, returning the parsed
    /// result if successful.
    ///
    /// This function can be called after a call to `accept_overlapped` has
    /// succeeded to parse out the data that was written in.
    pub fn parse(&self, socket: &TcpListener) -> io::Result<AcceptAddrs> {
        let mut ret = AcceptAddrs {
            local: 0 as *mut _, local_len: 0,
            remote: 0 as *mut _, remote_len: 0,
            _data: self,
        };
        let ptr = try!(GETACCEPTEXSOCKADDRS.get(socket.as_raw_socket()));
        assert!(ptr != 0);
        unsafe {
            let get_sockaddrs = mem::transmute::<_, GetAcceptExSockaddrs>(ptr);
            let (a, b, c, d) = self.args();
            get_sockaddrs(a, b, c, d,
                          &mut ret.local, &mut ret.local_len,
                          &mut ret.remote, &mut ret.remote_len);
            Ok(ret)
        }
    }

    fn args(&self) -> (PVOID, DWORD, DWORD, DWORD) {
        let remote_offset = unsafe {
            &(*(0 as *const AcceptAddrsBuf)).remote as *const _ as usize
        };
        (self as *const _ as *mut _, 0, remote_offset as DWORD,
         (mem::size_of_val(self) - remote_offset) as DWORD)
    }
}

impl<'a> AcceptAddrs<'a> {
    /// Returns the local socket address contained in this buffer.
    pub fn local(&self) -> Option<SocketAddr> {
        unsafe { ptrs_to_socket_addr(self.local, self.local_len) }
    }

    /// Returns the remote socket address contained in this buffer.
    pub fn remote(&self) -> Option<SocketAddr> {
        unsafe { ptrs_to_socket_addr(self.remote, self.remote_len) }
    }
}

impl WsaExtension {
    fn get(&self, socket: SOCKET) -> io::Result<usize> {
        let prev = self.val.load(Ordering::SeqCst);
        if prev != 0 && !cfg!(debug_assertions) {
            return Ok(prev)
        }
        let mut ret = 0 as usize;
        let mut bytes = 0;
        let r = unsafe {
            WSAIoctl(socket, SIO_GET_EXTENSION_FUNCTION_POINTER,
                     &self.guid as *const _ as *mut _,
                     mem::size_of_val(&self.guid) as DWORD,
                     &mut ret as *mut _ as *mut _,
                     mem::size_of_val(&ret) as DWORD,
                     &mut bytes,
                     0 as *mut _, None)
        };
        cvt(r).map(|_| {
            debug_assert_eq!(bytes as usize, mem::size_of_val(&ret));
            debug_assert!(prev == 0 || prev == ret);
            self.val.store(ret, Ordering::SeqCst);
            ret
        })

    }
}

#[cfg(test)]
mod tests {
    use std::net::{TcpListener, UdpSocket, TcpStream, SocketAddr};
    use std::thread;
    use std::io::prelude::*;
    use winapi::*;

    use iocp::CompletionPort;
    use net::{TcpStreamExt, UdpSocketExt, SocketAddrBuf};
    use net::{TcpBuilderExt, TcpListenerExt, AcceptAddrsBuf};
    use net2::TcpBuilder;

    fn overlapped() -> WSAOVERLAPPED {
        WSAOVERLAPPED {
            Internal: 0,
            InternalHigh: 0,
            Offset: 0,
            OffsetHigh: 0,
            hEvent: 0 as *mut _,
        }
    }

    fn each_ip(f: &mut FnMut(SocketAddr)) {
        f(t!("127.0.0.1:0".parse()));
        f(t!("[::1]:0".parse()));
    }

    #[test]
    fn tcp_read() {
        each_ip(&mut |addr| {
            let l = t!(TcpListener::bind(addr));
            let addr = t!(l.local_addr());
            let t = thread::spawn(move || {
                let mut a = t!(l.accept()).0;
                t!(a.write_all(&[1, 2, 3]));
            });

            let cp = t!(CompletionPort::new(1));
            let s = t!(TcpStream::connect(addr));
            t!(cp.add_socket(1, &s));

            let mut b = [0; 10];
            let mut a = overlapped();
            unsafe {
                t!(s.read_overlapped(&mut b, &mut a));
            }
            let status = t!(cp.get(None));
            assert_eq!(status.bytes_transferred(), 3);
            assert_eq!(status.token(), 1);
            assert_eq!(status.overlapped(), &mut a as *mut _);
            assert_eq!(&b[0..3], &[1, 2, 3]);

            t!(t.join());
        })
    }

    #[test]
    fn tcp_write() {
        each_ip(&mut |addr| {
            let l = t!(TcpListener::bind(addr));
            let addr = t!(l.local_addr());
            let t = thread::spawn(move || {
                let mut a = t!(l.accept()).0;
                let mut b = [0; 10];
                let n = t!(a.read(&mut b));
                assert_eq!(n, 3);
                assert_eq!(&b[0..3], &[1, 2, 3]);
            });

            let cp = t!(CompletionPort::new(1));
            let s = t!(TcpStream::connect(addr));
            t!(cp.add_socket(1, &s));

            let b = [1, 2, 3];
            let mut a = overlapped();
            unsafe {
                t!(s.write_overlapped(&b, &mut a));
            }
            let status = t!(cp.get(None));
            assert_eq!(status.bytes_transferred(), 3);
            assert_eq!(status.token(), 1);
            assert_eq!(status.overlapped(), &mut a as *mut _);

            t!(t.join());
        })
    }

    #[test]
    fn tcp_connect() {
        each_ip(&mut |addr_template| {
            let l = t!(TcpListener::bind(addr_template));
            let addr = t!(l.local_addr());
            let t = thread::spawn(move || {
                t!(l.accept());
            });

            let cp = t!(CompletionPort::new(1));
            let builder = match addr {
                SocketAddr::V4(..) => t!(TcpBuilder::new_v4()),
                SocketAddr::V6(..) => t!(TcpBuilder::new_v6()),
            };
            t!(cp.add_socket(1, &builder));

            let mut a = overlapped();
            t!(builder.bind(addr_template));
            let (_s, _) = unsafe {
                t!(builder.connect_overlapped(&addr, &mut a))
            };
            let status = t!(cp.get(None));
            assert_eq!(status.bytes_transferred(), 0);
            assert_eq!(status.token(), 1);
            assert_eq!(status.overlapped(), &mut a as *mut _);

            t!(t.join());
        })
    }

    #[test]
    fn udp_recv_from() {
        each_ip(&mut |addr| {
            let a = t!(UdpSocket::bind(addr));
            let b = t!(UdpSocket::bind(addr));
            let a_addr = t!(a.local_addr());
            let b_addr = t!(b.local_addr());
            let t = thread::spawn(move || {
                t!(a.send_to(&[1, 2, 3], b_addr));
            });

            let cp = t!(CompletionPort::new(1));
            t!(cp.add_socket(1, &b));

            let mut buf = [0; 10];
            let mut a = overlapped();
            let mut addr = SocketAddrBuf::new();
            unsafe {
                t!(b.recv_from_overlapped(&mut buf, &mut addr, &mut a));
            }
            let status = t!(cp.get(None));
            assert_eq!(status.bytes_transferred(), 3);
            assert_eq!(status.token(), 1);
            assert_eq!(status.overlapped(), &mut a as *mut _);
            assert_eq!(&buf[..3], &[1, 2, 3]);
            assert_eq!(addr.to_socket_addr(), Some(a_addr));

            t!(t.join());
        })
    }

    #[test]
    fn udp_send_to() {
        each_ip(&mut |addr| {
            let a = t!(UdpSocket::bind(addr));
            let b = t!(UdpSocket::bind(addr));
            let a_addr = t!(a.local_addr());
            let b_addr = t!(b.local_addr());
            let t = thread::spawn(move || {
                let mut b = [0; 100];
                let (n, addr) = t!(a.recv_from(&mut b));
                assert_eq!(n, 3);
                assert_eq!(addr, b_addr);
                assert_eq!(&b[..3], &[1, 2, 3]);
            });

            let cp = t!(CompletionPort::new(1));
            t!(cp.add_socket(1, &b));

            let mut a = overlapped();
            unsafe {
                t!(b.send_to_overlapped(&[1, 2, 3], &a_addr, &mut a));
            }
            let status = t!(cp.get(None));
            assert_eq!(status.bytes_transferred(), 3);
            assert_eq!(status.token(), 1);
            assert_eq!(status.overlapped(), &mut a as *mut _);

            t!(t.join());
        })
    }

    #[test]
    fn tcp_accept() {
        each_ip(&mut |addr_template| {
            let l = t!(TcpListener::bind(addr_template));
            let addr = t!(l.local_addr());
            let t = thread::spawn(move || {
                let socket = t!(TcpStream::connect(addr));
                (socket.local_addr().unwrap(), socket.peer_addr().unwrap())
            });

            let cp = t!(CompletionPort::new(1));
            let builder = match addr {
                SocketAddr::V4(..) => t!(TcpBuilder::new_v4()),
                SocketAddr::V6(..) => t!(TcpBuilder::new_v6()),
            };
            t!(cp.add_socket(1, &l));

            let mut a = overlapped();
            let mut addrs = AcceptAddrsBuf::new();
            let (_s, _) = unsafe {
                t!(l.accept_overlapped(&builder, &mut addrs, &mut a))
            };
            let status = t!(cp.get(None));
            assert_eq!(status.bytes_transferred(), 0);
            assert_eq!(status.token(), 1);
            assert_eq!(status.overlapped(), &mut a as *mut _);

            let (remote, local) = t!(t.join());
            let addrs = addrs.parse(&l).unwrap();
            assert_eq!(addrs.local(), Some(local));
            assert_eq!(addrs.remote(), Some(remote));
        })
    }
}
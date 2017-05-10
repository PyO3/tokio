# Copied from the uvloop project.  If you add a new unittest here,
# please consider contributing it to the uvloop project.
#
# Portions copyright (c) 2015-present MagicStack Inc.  http://magic.io

import asyncio
import socket
import sys

import pytest
import uvloop
import _testbase as tb


class MyBaseProto(asyncio.Protocol):
    connected = None
    done = None

    def __init__(self, loop=None):
        self.transport = None
        self.state = 'INITIAL'
        self.nbytes = 0
        if loop is not None:
            self.connected = asyncio.Future(loop=loop)
            self.done = asyncio.Future(loop=loop)

    def connection_made(self, transport):
        self.transport = transport
        assert self.state == 'INITIAL', self.state
        self.state = 'CONNECTED'
        if self.connected:
            self.connected.set_result(None)

    def data_received(self, data):
        assert self.state == 'CONNECTED', self.state
        self.nbytes += len(data)

    def eof_received(self):
        assert self.state == 'CONNECTED', self.state
        self.state = 'EOF'

    def connection_lost(self, exc):
        assert self.state in ('CONNECTED', 'EOF'), self.state
        self.state = 'CLOSED'
        if self.done:
            self.done.set_result(None)


@pytest.mark.skipif(
    sys.version_info[:3] == (3, 5, 2),
    reason='See https://github.com/python/asyncio/pull/366 for details')
def test_create_server_1(loop):
    CNT = 0           # number of clients that were successful
    TOTAL_CNT = 25    # total number of clients that test will create
    TIMEOUT = 5.0     # timeout for this test

    A_DATA = b'A' * 1024 * 1024
    B_DATA = b'B' * 1024 * 1024

    async def handle_client(reader, writer):
        nonlocal CNT

        data = await reader.readexactly(len(A_DATA))
        assert data == A_DATA
        writer.write(b'OK')

        data = await reader.readexactly(len(B_DATA))
        assert data == B_DATA
        writer.writelines([b'S', b'P'])
        writer.write(bytearray(b'A'))
        writer.write(memoryview(b'M'))

        await writer.drain()
        writer.close()

        CNT += 1

    async def test_client(addr):
        sock = socket.socket()
        with sock:
            sock.setblocking(False)
            await loop.sock_connect(sock, addr)

            await loop.sock_sendall(sock, A_DATA)

            buf = b''
            while len(buf) != 2:
                buf += await loop.sock_recv(sock, 1)

            assert buf == b'OK'

            await loop.sock_sendall(sock, B_DATA)

            buf = b''
            while len(buf) != 4:
                buf += await loop.sock_recv(sock, 1)
            assert buf == b'SPAM'

    async def start_server():
        nonlocal CNT
        CNT = 0

        addrs = ('127.0.0.1',  'localhost')
        if not isinstance(loop, uvloop.Loop):
            # Hack to let tests run on Python 3.5.0
            # (asyncio doesn't support multiple hosts in 3.5.0)
            addrs = '127.0.0.1'

        srv = await asyncio.start_server(
            handle_client,
            addrs, 0,
            family=socket.AF_INET,
            loop=loop)

        srv_socks = srv.sockets
        assert srv_socks

        addr = srv_socks[0].getsockname()

        tasks = []
        for _ in range(TOTAL_CNT):
            tasks.append(test_client(addr))

        await asyncio.wait_for(
            asyncio.gather(*tasks, loop=loop), TIMEOUT, loop=loop)

        loop.call_soon(srv.close)
        await srv.wait_closed()

        # Check that the server cleaned-up proxy-sockets
        for srv_sock in srv_socks:
            assert srv_sock.fileno() == -1

    async def start_server_sock():
        nonlocal CNT
        CNT = 0

        sock = socket.socket()
        sock.bind(('127.0.0.1', 0))
        addr = sock.getsockname()

        srv = await asyncio.start_server(
            handle_client,
            None, None,
            family=socket.AF_INET,
            loop=loop,
            sock=sock)

        srv_socks = srv.sockets
        assert srv_socks

        tasks = []
        for _ in range(TOTAL_CNT):
            tasks.append(test_client(addr))

        await asyncio.wait_for(
            asyncio.gather(*tasks, loop=loop),
            TIMEOUT, loop=loop)

        srv.close()
        await srv.wait_closed()

        # Check that the server cleaned-up proxy-sockets
        for srv_sock in srv_socks:
            assert srv_sock.fileno() == -1

    loop.run_until_complete(start_server())
    assert CNT == TOTAL_CNT

    loop.run_until_complete(start_server_sock())
    assert CNT == TOTAL_CNT


def test_create_server_2(loop):
    with pytest.raises(ValueError) as excinfo:
        loop.run_until_complete(loop.create_server(object))
    excinfo.match('nor sock were specified')


def test_create_server_3(loop):
    ''' check ephemeral port can be used '''

    async def start_server_ephemeral_ports():

        for port_sentinel in [0, None]:
            srv = await loop.create_server(
                asyncio.Protocol,
                '127.0.0.1', port_sentinel,
                family=socket.AF_INET)

            srv_socks = srv.sockets
            assert srv_socks

            host, port = srv_socks[0].getsockname()
            assert port != 0

            loop.call_soon(srv.close)
            await srv.wait_closed()

            # Check that the server cleaned-up proxy-sockets
            for srv_sock in srv_socks:
                assert srv_sock.fileno() == -1

    loop.run_until_complete(start_server_ephemeral_ports())


def test_create_server_4(loop):
    sock = socket.socket()
    sock.bind(('127.0.0.1', 0))

    with sock:
        addr = sock.getsockname()

        with pytest.raises(OSError) as excinfo:
            loop.run_until_complete(loop.create_server(object, *addr))

        excinfo.match("in use")


def test_create_server_5(loop, port):
    # Test that create_server sets the TCP_IPV6ONLY flag,
    # so it can bind to ipv4 and ipv6 addresses
    # simultaneously.

    async def runner():
        srv = await loop.create_server(asyncio.Protocol, None, port)
        srv.close()
        await srv.wait_closed()

    loop.run_until_complete(runner())


@pytest.mark.skipif(not hasattr(socket, 'SO_REUSEPORT'),
                    reason='The system does not support SO_REUSEPORT')
@pytest.mark.skipif(sys.version_info[:3] < (3, 5, 1),
                    reason='asyncio in CPython 3.5.0 does not have the '
                    'reuse_port argument')
def test_create_server_6(loop, port):

    async def runner():
        srv1 = await loop.create_server(
            asyncio.Protocol,
            None, port,
            reuse_port=True)

        srv2 = await loop.create_server(
            asyncio.Protocol,
            None, port,
            reuse_port=True)

        srv1.close()
        srv2.close()

        await srv1.wait_closed()
        await srv2.wait_closed()

    loop.run_until_complete(runner())


def test_create_connection_1(loop):
    CNT = 0
    TOTAL_CNT = 100

    def server():
        data = yield tb.read(4)
        assert data == b'AAAA'
        yield tb.write(b'OK')

        data = yield tb.read(4)
        assert data == b'BBBB'
        yield tb.write(b'SPAM')

    async def client(addr):
        reader, writer = await asyncio.open_connection(
            *addr,
            loop=loop)

        writer.write(b'AAAA')
        assert (await reader.readexactly(2)) == b'OK'

        # with pytest.raises(TypeError) as excinfo:
        #    writer.write('AAAA')
        # excinfo.match(r'(a bytes-like object)|(must be byte-ish)')

        writer.write(b'BBBB')
        assert (await reader.readexactly(4)) == b'SPAM'

        nonlocal CNT
        CNT += 1

        writer.close()

    async def client_2(addr):
        sock = socket.socket()
        sock.connect(addr)
        reader, writer = await asyncio.open_connection(
            sock=sock,
            loop=loop)

        writer.write(b'AAAA')
        assert (await reader.readexactly(2)) == b'OK'

        writer.write(b'BBBB')
        assert (await reader.readexactly(4)) == b'SPAM'

        nonlocal CNT
        CNT += 1

        writer.close()

    def run(coro):
        nonlocal CNT
        CNT = 0

        srv = tb.tcp_server(server,
                            max_clients=TOTAL_CNT,
                            backlog=TOTAL_CNT)
        srv.start()

        tasks = []
        for _ in range(TOTAL_CNT):
            tasks.append(coro(srv.addr))

        loop.run_until_complete(asyncio.gather(*tasks, loop=loop))
        srv.join()
        assert CNT == TOTAL_CNT

    run(client)
    run(client_2)


def test_create_connection_2(loop):
    sock = socket.socket()
    with sock:
        sock.bind(('127.0.0.1', 0))
        addr = sock.getsockname()

    async def client():
        reader, writer = await asyncio.open_connection(
            *addr,
            loop=loop)

    async def runner():
        with pytest.raises(ConnectionRefusedError):
            await client()

    loop.run_until_complete(runner())


def test_create_connection_3(loop):
    CNT = 0
    TOTAL_CNT = 100

    def server():
        data = yield tb.read(4)
        assert data == b'AAAA'
        yield tb.close()

    async def client(addr):
        reader, writer = await asyncio.open_connection(*addr, loop=loop)
        writer.write(b'AAAA')

        with pytest.raises(asyncio.IncompleteReadError):
            await reader.readexactly(10)

        writer.close()

        nonlocal CNT
        CNT += 1

    def run(coro):
        nonlocal CNT
        CNT = 0

        srv = tb.tcp_server(server,
                            max_clients=TOTAL_CNT,
                            backlog=TOTAL_CNT)
        srv.start()

        tasks = []
        for _ in range(TOTAL_CNT):
            tasks.append(coro(srv.addr))

        loop.run_until_complete(
            asyncio.gather(*tasks, loop=loop))
        srv.join()
        assert CNT == TOTAL_CNT

    run(client)


def test_create_connection_4(loop):
    sock = socket.socket()
    sock.close()

    async def client():
        reader, writer = await asyncio.open_connection(sock=sock, loop=loop)

    async def runner():
        with pytest.raises(OSError) as excinfo:
            await client()
        excinfo.match('Bad file')

    loop.run_until_complete(runner())


def test_transport_shutdown(loop):
    CNT = 0           # number of clients that were successful
    TOTAL_CNT = 100   # total number of clients that test will create
    TIMEOUT = 5.0     # timeout for this test

    async def handle_client(reader, writer):
        nonlocal CNT

        data = await reader.readexactly(4)
        assert data == b'AAAA'

        writer.write(b'OK')
        writer.write_eof()
        writer.write_eof()

        await writer.drain()
        writer.close()

        CNT += 1

    async def test_client(addr):
        reader, writer = await asyncio.open_connection(*addr, loop=loop)

        writer.write(b'AAAA')
        data = await reader.readexactly(2)
        assert data == b'OK'

        writer.close()

    async def start_server():
        nonlocal CNT
        CNT = 0

        srv = await asyncio.start_server(
            handle_client,
            '127.0.0.1', 0,
            family=socket.AF_INET,
            loop=loop)

        srv_socks = srv.sockets
        assert srv_socks

        addr = srv_socks[0].getsockname()

        tasks = []
        for _ in range(TOTAL_CNT):
            tasks.append(test_client(addr))

        await asyncio.wait_for(
            asyncio.gather(*tasks, loop=loop),
            TIMEOUT, loop=loop)

        srv.close()
        await srv.wait_closed()

    loop.run_until_complete(start_server())
    assert CNT == TOTAL_CNT


def test_tcp_handle_exception_in_connection_made(loop):
    # Test that if connection_made raises an exception,
    # 'create_connection' still returns.

    # Silence error logging
    loop.set_exception_handler(lambda *args: None)

    fut = asyncio.Future(loop=loop)
    connection_lost_called = asyncio.Future(loop=loop)

    async def server(reader, writer):
        try:
            await reader.read()
        finally:
            writer.close()

    class Proto(asyncio.Protocol):
        def connection_made(self, tr):
            1 / 0

        def connection_lost(self, exc):
            connection_lost_called.set_result(exc)

    srv = loop.run_until_complete(asyncio.start_server(
        server,
        '127.0.0.1', 0,
        family=socket.AF_INET,
        loop=loop))

    async def runner():
        tr, pr = await asyncio.wait_for(
            loop.create_connection(
                Proto, *srv.sockets[0].getsockname()),
            timeout=1.0, loop=loop)
        fut.set_result(None)
        tr.close()

    loop.run_until_complete(runner())
    srv.close()
    loop.run_until_complete(srv.wait_closed())
    loop.run_until_complete(fut)

    assert loop.run_until_complete(connection_lost_called) is None


def test_many_small_writes(loop):
    N = 10000
    TOTAL = 0

    fut = loop.create_future()

    async def server(reader, writer):
        nonlocal TOTAL
        while True:
            d = await reader.read(10000)
            if not d:
                break
            TOTAL += len(d)
        fut.set_result(True)
        writer.close()

    async def run():
        srv = await asyncio.start_server(
            server,
            '127.0.0.1', 0,
            family=socket.AF_INET,
            loop=loop)

        addr = srv.sockets[0].getsockname()
        r, w = await asyncio.open_connection(*addr, loop=loop)

        DATA = b'x' * 102400

        # Test _StreamWriteContext with short sequences of writes
        w.write(DATA)
        await w.drain()
        for _ in range(3):
            w.write(DATA)
            await w.drain()
        for _ in range(10):
            w.write(DATA)
            await w.drain()

        for _ in range(N):
            w.write(DATA)

            try:
                w.write('a')
            except TypeError:
                pass

        await w.drain()
        for _ in range(N):
            w.write(DATA)
            await w.drain()

        w.close()
        await fut

        srv.close()
        await srv.wait_closed()

        assert TOTAL == N * 2 * len(DATA) + 14 * len(DATA)

    loop.run_until_complete(run())


@pytest.mark.skipif(not hasattr(socket, 'AF_UNIX'), reason='no Unix sockets')
def _test_create_connection_wrong_sock(loop):
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    with sock:
        coro = loop.create_connection(MyBaseProto, sock=sock)
        with pytest.raises(ValueError) as excinfo:
            loop.run_until_complete(coro)

        excinfo.match('A Stream Socket was expected')


@pytest.mark.skipif(not hasattr(socket, 'AF_UNIX'), reason='no Unix sockets')
def test_create_server_wrong_sock(loop):
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    with sock:
        with pytest.raises(ValueError) as excinfo:
            coro = loop.create_server(MyBaseProto, sock=sock)
            loop.run_until_complete(coro)

        excinfo.match('A Stream Socket was expected')


@pytest.mark.skipif(not hasattr(socket, 'SOCK_NONBLOCK'),
                    reason='no socket.SOCK_NONBLOCK (linux only)')
def test_create_server_stream_bittype(loop):
    sock = socket.socket(
        socket.AF_INET, socket.SOCK_STREAM | socket.SOCK_NONBLOCK)
    with sock:
        coro = loop.create_server(lambda: None, sock=sock)
        srv = loop.run_until_complete(coro)
        srv.close()
        loop.run_until_complete(srv.wait_closed())

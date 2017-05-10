# Copied from the uvloop project.  If you add a new unittest here,
# please consider contributing it to the uvloop project.
#
# Portions copyright (c) 2015-present MagicStack Inc.  http://magic.io

import asyncio
import os
import socket
import tempfile

import pytest
import _testbase as tb

from test_ssl import (ONLYCERT, ONLYKEY, create_client_ssl_context,
                      create_server_ssl_context)


def test_create_unix_server_1(loop):
    CNT = 0           # number of clients that were successful
    TOTAL_CNT = 100   # total number of clients that test will create
    TIMEOUT = 5.0     # timeout for this test

    async def handle_client(reader, writer):
        nonlocal CNT

        data = await reader.readexactly(4)
        assert data == b'AAAA'
        writer.write(b'OK')

        data = await reader.readexactly(4)
        assert data == b'BBBB'
        writer.write(b'SPAM')

        await writer.drain()
        writer.close()

        CNT += 1

    async def test_client(addr):
        sock = socket.socket(socket.AF_UNIX)
        with sock:
            sock.setblocking(False)
            await loop.sock_connect(sock, addr)

            await loop.sock_sendall(sock, b'AAAA')

            buf = b''
            while len(buf) != 2:
                buf += await loop.sock_recv(sock, 1)
            assert buf == b'OK'

            await loop.sock_sendall(sock, b'BBBB')

            buf = b''
            while len(buf) != 4:
                buf += await loop.sock_recv(sock, 1)
            assert buf == b'SPAM'

    async def start_server():
        nonlocal CNT
        CNT = 0

        with tempfile.TemporaryDirectory() as td:
            sock_name = os.path.join(td, 'sock')
            srv = await asyncio.start_unix_server(
                handle_client,
                sock_name,
                loop=loop)

            try:
                # srv_socks = srv.sockets
                # assert srv_socks

                tasks = []
                for _ in range(TOTAL_CNT):
                    tasks.append(test_client(sock_name))

                await asyncio.wait_for(
                    asyncio.gather(*tasks, loop=loop),
                    TIMEOUT, loop=loop)

            finally:
                loop.call_soon(srv.close)
                await srv.wait_closed()

                # Check that the server cleaned-up proxy-sockets
                # for srv_sock in srv_socks:
                #    assert srv_sock.fileno() == -1

            # asyncio doesn't cleanup the sock file
            assert os.path.exists(sock_name)

    async def start_server_sock(start_server):
        nonlocal CNT
        CNT = 0

        with tempfile.TemporaryDirectory() as td:
            sock_name = os.path.join(td, 'sock')
            sock = socket.socket(socket.AF_UNIX)
            sock.bind(sock_name)

            srv = await start_server(sock)
            await asyncio.sleep(0.1, loop=loop)

            try:
                # srv_socks = srv.sockets
                # self.assertTrue(srv_socks)

                tasks = []
                for _ in range(TOTAL_CNT):
                    tasks.append(test_client(sock_name))

                await asyncio.wait_for(
                    asyncio.gather(*tasks, loop=loop),
                    TIMEOUT, loop=loop)

            finally:
                loop.call_soon(srv.close)
                await srv.wait_closed()

                # Check that the server cleaned-up proxy-sockets
                # for srv_sock in srv_socks:
                #    self.assertEqual(srv_sock.fileno(), -1)

            # asyncio doesn't cleanup the sock file
            assert os.path.exists(sock_name)

    # with self.subTest(func='start_unix_server(host, port)'):
    loop.run_until_complete(start_server())
    assert CNT == TOTAL_CNT

    # with self.subTest(func='start_unix_server(sock)'):
    loop.run_until_complete(start_server_sock(
        lambda sock: asyncio.start_unix_server(
            handle_client,
            None,
            loop=loop,
            sock=sock)))
    assert CNT == TOTAL_CNT

    # with self.subTest(func='start_server(sock)'):
    loop.run_until_complete(start_server_sock(
        lambda sock: asyncio.start_server(
            handle_client,
            None, None,
            loop=loop,
            sock=sock)))
    assert CNT == TOTAL_CNT


def test_create_unix_server_2(loop):
    with tempfile.TemporaryDirectory() as td:
        sock_name = os.path.join(td, 'sock')
        with open(sock_name, 'wt') as f:
            f.write('x')

        with pytest.raises(OSError) as excinfo:
            loop.run_until_complete(
                loop.create_unix_server(object, sock_name))

        excinfo.match('in use')


def test_create_unix_connection_1(loop):
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
        reader, writer = await asyncio.open_unix_connection(
            addr,
            loop=loop)

        writer.write(b'AAAA')
        assert await reader.readexactly(2) == b'OK'

        writer.write(b'BBBB')
        assert await reader.readexactly(4) == b'SPAM'

        nonlocal CNT
        CNT += 1

        writer.close()

    async def client_2(addr):
        sock = socket.socket(socket.AF_UNIX)
        sock.connect(addr)
        reader, writer = await asyncio.open_unix_connection(
            sock=sock,
            loop=loop)

        writer.write(b'AAAA')
        assert await reader.readexactly(2) == b'OK'

        writer.write(b'BBBB')
        assert await reader.readexactly(4) == b'SPAM'

        nonlocal CNT
        CNT += 1

        writer.close()

    async def client_3(addr):
        sock = socket.socket(socket.AF_UNIX)
        sock.connect(addr)
        reader, writer = await asyncio.open_connection(
            sock=sock,
            loop=loop)

        writer.write(b'AAAA')
        assert await reader.readexactly(2) == b'OK'

        writer.write(b'BBBB')
        assert await reader.readexactly(4) == b'SPAM'

        nonlocal CNT
        CNT += 1

        writer.close()

    def run(coro):
        nonlocal CNT
        CNT = 0

        srv = tb.tcp_server(server,
                            family=socket.AF_UNIX,
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
    # run(client_2)
    # run(client_3)


def test_create_unix_connection_2(loop):
    with tempfile.NamedTemporaryFile() as tmp:
        path = tmp.name

    async def client():
        reader, writer = await asyncio.open_unix_connection(
            path,
            loop=loop)

    async def runner():
        with pytest.raises(FileNotFoundError):
            await client()

    loop.run_until_complete(runner())


def test_create_unix_connection_3(loop):
    CNT = 0
    TOTAL_CNT = 100

    def server():
        data = yield tb.read(4)
        assert data == b'AAAA'
        yield tb.close()

    async def client(addr):
        reader, writer = await asyncio.open_unix_connection(
            addr,
            loop=loop)

        # sock = writer._transport.get_extra_info('socket')
        # assert sock.family == socket.AF_UNIX

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
                            family=socket.AF_UNIX,
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


def test_create_unix_connection_4(loop):
    sock = socket.socket(socket.AF_UNIX)
    sock.close()

    async def client():
        reader, writer = await asyncio.open_unix_connection(
            sock=sock,
            loop=loop)

    async def runner():
        with pytest.raises(OSError) as excinfo:
            await client()

        excinfo.match('Bad file')

    loop.run_until_complete(runner())


def test_create_unix_connection_5(loop):
    s1, s2 = socket.socketpair(socket.AF_UNIX)

    excs = []

    class Proto(asyncio.Protocol):
        def connection_lost(self, exc):
            excs.append(exc)

    proto = Proto()

    async def client():
        t, _ = await loop.create_unix_connection(
            lambda: proto,
            None,
            sock=s2)

        t.write(b'AAAAA')
        s1.close()
        t.write(b'AAAAA')
        await asyncio.sleep(0.1, loop=loop)

    loop.run_until_complete(client())

    assert len(excs) == 1
    assert excs[0].__class__ in (BrokenPipeError, ConnectionResetError)


def test_create_unix_connection_ssl_1(loop):
    CNT = 0
    TOTAL_CNT = 25

    A_DATA = b'A' * 1024 * 1024
    B_DATA = b'B' * 1024 * 1024

    sslctx = create_server_ssl_context(ONLYCERT, ONLYKEY)
    client_sslctx = create_client_ssl_context()

    def server():
        yield tb.starttls(
            sslctx,
            server_side=True)

        data = yield tb.read(len(A_DATA))
        assert data == A_DATA
        yield tb.write(b'OK')

        data = yield tb.read(len(B_DATA))
        assert data == B_DATA
        yield tb.write(b'SPAM')

        yield tb.close()

    async def client(addr):
        reader, writer = await asyncio.open_unix_connection(
            addr,
            ssl=client_sslctx,
            server_hostname='',
            loop=loop)

        writer.write(A_DATA)
        assert await reader.readexactly(2) == b'OK'

        writer.write(B_DATA)
        assert await reader.readexactly(4) == b'SPAM'

        nonlocal CNT
        CNT += 1

        writer.close()

    def run(coro):
        nonlocal CNT
        CNT = 0

        srv = tb.tcp_server(server,
                            family=socket.AF_UNIX,
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

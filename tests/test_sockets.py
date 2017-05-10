# Copied from the uvloop project.  If you add a new unittest here,
# please consider contributing it to the uvloop project.
#
# Portions copyright (c) 2015-present MagicStack Inc.  http://magic.io

import asyncio
import socket
import sys

import pytest
import _testbase as tb


_SIZE = 1024 * 1024


async def recv_all(loop, sock, nbytes):
    buf = b''
    while len(buf) < nbytes:
        buf += await loop.sock_recv(sock, nbytes - len(buf))
    return buf


@pytest.mark.skipif(
    sys.version_info[:3] == (3, 5, 2),
    reason='See https://github.com/python/asyncio/pull/366 for details')
def test_socket_connect_recv_send(loop):
    def srv_gen():
        yield tb.write(b'helo')
        data = yield tb.read(4 * _SIZE)
        assert data == b'ehlo' * _SIZE
        yield tb.write(b'O')
        yield tb.write(b'K')

    # We use @asyncio.coroutine & `yield from` to test
    # the compatibility of Cython's 'async def' coroutines.
    @asyncio.coroutine
    def client(sock, addr):
        yield from loop.sock_connect(sock, addr)
        data = yield from recv_all(loop, sock, 4)
        assert data == b'helo'
        yield from loop.sock_sendall(sock, b'ehlo' * _SIZE)
        data = yield from recv_all(loop, sock, 2)
        assert data == b'OK'

    with tb.tcp_server(srv_gen) as srv:
        sock = socket.socket()
        with sock:
            sock.setblocking(False)
            loop.run_until_complete(client(sock, srv.addr))


def test_socket_accept_recv_send(loop):
    async def server():
        sock = socket.socket()
        sock.setblocking(False)

        with sock:
            sock.bind(('127.0.0.1', 0))
            sock.listen()

            fut = loop.run_in_executor(None, client, sock.getsockname())

            client_sock, _ = await loop.sock_accept(sock)

            with client_sock:
                data = await recv_all(loop, client_sock, _SIZE)
                assert data == b'a' * _SIZE

            await fut

    def client(addr):
        sock = socket.socket()
        with sock:
            sock.connect(addr)
            sock.sendall(b'a' * _SIZE)

    loop.run_until_complete(server())


def test_socket_failed_connect(loop):
    sock = socket.socket()
    with sock:
        sock.bind(('127.0.0.1', 0))
        addr = sock.getsockname()

    async def run():
        sock = socket.socket()
        with sock:
            sock.setblocking(False)
            with pytest.raises(ConnectionRefusedError):
                await loop.sock_connect(sock, addr)

    loop.run_until_complete(run())


def test_socket_blocking_error(loop):
    loop.set_debug(True)
    sock = socket.socket()

    with sock:
        with pytest.raises(ValueError) as excinfo:
            loop.run_until_complete(loop.sock_recv(sock, 0))

        excinfo.match('must be non-blocking')

        with pytest.raises(ValueError) as excinfo:
            loop.run_until_complete(loop.sock_sendall(sock, b''))

        excinfo.match('must be non-blocking')

        with pytest.raises(ValueError) as excinfo:
            loop.run_until_complete(loop.sock_accept(sock))

        excinfo.match('must be non-blocking')

        with pytest.raises(ValueError) as excinfo:
            loop.run_until_complete(loop.sock_connect(sock, (b'', 0)))

        excinfo.match('must be non-blocking')

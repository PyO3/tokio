# Copied from the uvloop project.  If you add a new unittest here,
# please consider contributing it to the uvloop project.
#
# Portions copyright (c) 2015-present MagicStack Inc.  http://magic.io

import asyncio
import socket
import ssl

import _testbase as tb


ONLYCERT = tb._cert_fullname('ssl_cert.pem')
ONLYKEY = tb._cert_fullname('ssl_key.pem')


def create_server_ssl_context(certfile, keyfile=None):
    sslcontext = ssl.SSLContext(ssl.PROTOCOL_SSLv23)
    sslcontext.options |= ssl.OP_NO_SSLv2
    sslcontext.load_cert_chain(certfile, keyfile)
    return sslcontext


def create_client_ssl_context():
    sslcontext = ssl.create_default_context()
    sslcontext.check_hostname = False
    sslcontext.verify_mode = ssl.CERT_NONE
    return sslcontext


def test_create_server_ssl_1(loop):
    CNT = 0           # number of clients that were successful
    TOTAL_CNT = 25    # total number of clients that test will create
    TIMEOUT = 10.0    # timeout for this test

    A_DATA = b'A' * 1024 * 1024
    B_DATA = b'B' * 1024 * 1024

    sslctx = create_server_ssl_context(ONLYCERT, ONLYKEY)
    client_sslctx = create_client_ssl_context()

    clients = []

    async def handle_client(reader, writer):
        nonlocal CNT

        data = await reader.readexactly(len(A_DATA))
        # assert data == A_DATA
        writer.write(b'OK')

        data = await reader.readexactly(len(B_DATA))
        assert data == B_DATA
        writer.writelines([b'SP', bytearray(b'A'), memoryview(b'M')])

        await writer.drain()
        writer.close()

        CNT += 1

    async def test_client(addr):
        fut = asyncio.Future(loop=loop)

        def prog():
            try:
                yield tb.starttls(client_sslctx)
                yield tb.connect(addr)
                yield tb.write(A_DATA)

                data = yield tb.read(2)
                assert data == b'OK'

                yield tb.write(B_DATA)
                data = yield tb.read(4)
                assert data == b'SPAM'

                yield tb.close()

            except Exception as ex:
                loop.call_soon_threadsafe(fut.set_exception, ex)
            else:
                loop.call_soon_threadsafe(fut.set_result, None)

        client = tb.tcp_client(prog)
        client.start()
        clients.append(client)

        await fut

    async def start_server():
        srv = await asyncio.start_server(
            handle_client,
            '127.0.0.1', 0,
            family=socket.AF_INET,
            ssl=sslctx,
            loop=loop)

        try:
            srv_socks = srv.sockets
            assert srv_socks

            addr = srv_socks[0].getsockname()

            tasks = []
            for _ in range(TOTAL_CNT):
                tasks.append(test_client(addr))

            await asyncio.wait_for(
                asyncio.gather(*tasks, loop=loop),
                TIMEOUT, loop=loop)
        finally:
            loop.call_soon(srv.close)
            await srv.wait_closed()

    loop.run_until_complete(start_server())

    assert CNT == TOTAL_CNT

    for client in clients:
        client.stop()


def test_create_connection_ssl_1(loop):
    CNT = 0
    TOTAL_CNT = 1

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
        reader, writer = await asyncio.open_connection(
            *addr,
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

    async def client_sock(addr):
        sock = socket.socket()
        sock.connect(addr)
        reader, writer = await asyncio.open_connection(
            sock=sock,
            ssl=client_sslctx,
            server_hostname='',
            loop=loop)

        writer.write(A_DATA)
        assert await reader.readexactly(2) == b'OK'

        writer.write(B_DATA)
        assert await reader.readexactly(4) == b'SPAM'

        nonlocal CNT
        CNT += 1

        try:
            writer.close()
            sock.close()
        except:
            import traceback
            traceback.print_exc()

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
    run(client_sock)

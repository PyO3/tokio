# Copied from the uvloop project.  If you add a new unittest here,
# please consider contributing it to the uvloop project.
#
# Portions copyright (c) 2015-present MagicStack Inc.  http://magic.io

import asyncio
import io
import os

from asyncio import test_utils


# All tests are copied from asyncio (mostly as-is)


class MyReadPipeProto(asyncio.Protocol):
    done = None

    def __init__(self, loop=None):
        self.state = ['INITIAL']
        self.nbytes = 0
        self.transport = None
        if loop is not None:
            self.done = asyncio.Future(loop=loop)

    def connection_made(self, transport):
        self.transport = transport
        assert self.state == ['INITIAL'], self.state
        self.state.append('CONNECTED')

    def data_received(self, data):
        assert self.state == ['INITIAL', 'CONNECTED'], self.state
        self.nbytes += len(data)

    def eof_received(self):
        assert self.state == ['INITIAL', 'CONNECTED'], self.state
        self.state.append('EOF')

    def connection_lost(self, exc):
        if 'EOF' not in self.state:
            self.state.append('EOF')  # It is okay if EOF is missed.
        assert self.state == ['INITIAL', 'CONNECTED', 'EOF'], self.state
        self.state.append('CLOSED')
        if self.done:
            self.done.set_result(None)


class MyWritePipeProto(asyncio.BaseProtocol):
    done = None

    def __init__(self, loop=None):
        self.state = 'INITIAL'
        self.transport = None
        if loop is not None:
            self.done = asyncio.Future(loop=loop)

    def connection_made(self, transport):
        self.transport = transport
        assert self.state == 'INITIAL', self.state
        self.state = 'CONNECTED'

    def connection_lost(self, exc):
        assert self.state == 'CONNECTED', self.state
        self.state = 'CLOSED'
        if self.done:
            self.done.set_result(None)


def test_read_pipe(loop):
    proto = MyReadPipeProto(loop=loop)

    rpipe, wpipe = os.pipe()
    pipeobj = io.open(rpipe, 'rb', 1024)

    @asyncio.coroutine
    def connect():
        t, p = yield from loop.connect_read_pipe(lambda: proto, pipeobj)
        assert p is proto
        assert t is proto.transport
        assert ['INITIAL', 'CONNECTED'] == proto.state
        assert 0 == proto.nbytes

    loop.run_until_complete(connect())

    os.write(wpipe, b'1')
    test_utils.run_until(loop, lambda: proto.nbytes >= 1)
    assert 1 == proto.nbytes

    os.write(wpipe, b'2345')
    test_utils.run_until(loop, lambda: proto.nbytes >= 5)
    assert ['INITIAL', 'CONNECTED'] == proto.state
    assert 5 == proto.nbytes

    os.close(wpipe)
    loop.run_until_complete(proto.done)
    assert ['INITIAL', 'CONNECTED', 'EOF', 'CLOSED'] == proto.state

    # extra info is available
    assert proto.transport.get_extra_info('pipe') is not None


def test_read_pty_output(loop):
    proto = MyReadPipeProto(loop=loop)

    master, slave = os.openpty()
    master_read_obj = io.open(master, 'rb', 0)

    @asyncio.coroutine
    def connect():
        t, p = yield from loop.connect_read_pipe(
            lambda: proto, master_read_obj)

        assert p is proto
        assert t is proto.transport
        assert ['INITIAL', 'CONNECTED'] == proto.state
        assert 0 == proto.nbytes

    loop.run_until_complete(connect())

    os.write(slave, b'1')
    test_utils.run_until(loop, lambda: proto.nbytes)
    assert 1 == proto.nbytes

    os.write(slave, b'2345')
    test_utils.run_until(loop, lambda: proto.nbytes >= 5)
    assert ['INITIAL', 'CONNECTED'] == proto.state
    assert 5 == proto.nbytes

    # On Linux, transport raises EIO when slave is closed --
    # ignore it.
    loop.set_exception_handler(lambda loop, ctx: None)
    os.close(slave)
    loop.run_until_complete(proto.done)

    assert ['INITIAL', 'CONNECTED', 'EOF', 'CLOSED'] == proto.state
    # extra info is available
    assert proto.transport.get_extra_info('pipe') is not None


def test_write_pipe(loop):
    rpipe, wpipe = os.pipe()
    os.set_blocking(rpipe, False)
    pipeobj = io.open(wpipe, 'wb', 1024)

    proto = MyWritePipeProto(loop=loop)
    connect = loop.connect_write_pipe(lambda: proto, pipeobj)
    transport, p = loop.run_until_complete(connect)
    assert p is proto
    assert transport is proto.transport
    assert 'CONNECTED' == proto.state

    transport.write(b'1')

    data = bytearray()

    def reader(data):
        try:
            chunk = os.read(rpipe, 1024)
        except BlockingIOError:
            return len(data)
        data += chunk
        return len(data)

    test_utils.run_until(loop, lambda: reader(data) >= 1)
    assert b'1' == data

    transport.write(b'2345')
    test_utils.run_until(loop, lambda: reader(data) >= 5)
    assert b'12345' == data
    assert 'CONNECTED' == proto.state

    os.close(rpipe)

    # extra info is available
    assert proto.transport.get_extra_info('pipe') is not None

    # close connection
    proto.transport.close()
    loop.run_until_complete(proto.done)
    assert 'CLOSED' == proto.state


def test_write_pipe_disconnect_on_close(loop):
    rsock, wsock = test_utils.socketpair()
    rsock.setblocking(False)

    pipeobj = io.open(wsock.detach(), 'wb', 1024)

    proto = MyWritePipeProto(loop=loop)
    connect = loop.connect_write_pipe(lambda: proto, pipeobj)
    transport, p = loop.run_until_complete(connect)
    assert p is proto
    assert transport is proto.transport
    assert 'CONNECTED' is proto.state

    transport.write(b'1')
    data = loop.run_until_complete(loop.sock_recv(rsock, 1024))
    assert b'1' == data

    rsock.close()

    loop.run_until_complete(proto.done)
    assert 'CLOSED' == proto.state


def test_write_pty(loop):
    master, slave = os.openpty()
    os.set_blocking(master, False)

    slave_write_obj = io.open(slave, 'wb', 0)

    proto = MyWritePipeProto(loop=loop)
    connect = loop.connect_write_pipe(lambda: proto, slave_write_obj)
    transport, p = loop.run_until_complete(connect)
    assert p is proto
    assert transport is proto.transport
    assert'CONNECTED' == proto.state

    transport.write(b'1')

    data = bytearray()

    def reader(data):
        try:
            chunk = os.read(master, 1024)
        except BlockingIOError:
            return len(data)
        data += chunk
        return len(data)

    test_utils.run_until(loop, lambda: reader(data) >= 1, timeout=10)
    assert b'1' == data

    transport.write(b'2345')
    test_utils.run_until(loop, lambda: reader(data) >= 5, timeout=10)
    assert b'12345' == data
    assert 'CONNECTED' == proto.state

    os.close(master)

    # extra info is available
    assert proto.transport.get_extra_info('pipe') is not None

    # close connection
    proto.transport.close()
    loop.run_until_complete(proto.done)
    assert 'CLOSED' == proto.state

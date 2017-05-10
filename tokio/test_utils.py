"""Utilities shared by tests."""
import asyncio
import contextlib
import functools
import gc
import socket
from unittest import mock

sentinel = ()


def run_briefly(loop):
    @asyncio.coroutine
    def once():
        pass
    t = asyncio.Task(once(), loop=loop)
    loop.run_until_complete(t)


def unused_port():
    """Return a port that is unused on the current host."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def unittest_run_loop(func, *args, **kwargs):
    """A decorator dedicated to use with asynchronous methods of an
    AioHTTPTestCase.

    Handles executing an asynchronous function, using
    the self.loop of the AioHTTPTestCase.
    """

    @functools.wraps(func, *args, **kwargs)
    def new_func(self):
        return self.loop.run_until_complete(func(self, *args, **kwargs))

    return new_func


@contextlib.contextmanager
def loop_context(loop_factory, fast=False):
    """A contextmanager that creates an event_loop, for test purposes.

    Handles the creation and cleanup of a test loop.
    """
    asyncio.set_event_loop_policy(loop_factory())
    loop = setup_test_loop(loop_factory)
    yield loop
    teardown_test_loop(loop, fast=fast)


def setup_test_loop(loop_factory=asyncio.new_event_loop):
    """Create and return an asyncio.BaseEventLoop
    instance.

    The caller should also call teardown_test_loop,
    once they are done with the loop.
    """
    # loop = loop_factory()
    loop = asyncio.new_event_loop()
    asyncio.set_event_loop(loop)
    return loop


def teardown_test_loop(loop, fast=False):
    """Teardown and cleanup an event_loop created
    by setup_test_loop.

    """
    closed = loop.is_closed()
    if not closed:
        loop.call_soon(loop.stop)
        loop.run_forever()
        loop.close()

    if not fast:
        gc.collect()

    asyncio.set_event_loop(None)


def make_mocked_coro(return_value=sentinel, raise_exception=sentinel):
    """Creates a coroutine mock."""
    @asyncio.coroutine
    def mock_coro(*args, **kwargs):
        if raise_exception is not sentinel:
            raise raise_exception
        return return_value

    return mock.Mock(wraps=mock_coro)

# Copied from the uvloop project.  If you add a new unittest here,
# please consider contributing it to the uvloop project.
#
# Portions copyright (c) 2015-present MagicStack Inc.  http://magic.io

import asyncio
import concurrent.futures
import re
import sys
import threading
from asyncio import test_utils
from test import support
from unittest import mock

import pytest


@pytest.fixture
def make_callback():
    # Create a callback function that appends thing to bag.
    def _make_callback(bag, thing):
        def bag_appender(future):
            bag.append(thing)
        return bag_appender

    yield _make_callback


def _fakefunc(f):
    return f


def first_cb():
    pass


def last_cb():
    pass


def test_future_initial_state(create_future):
    f = create_future()
    assert not f.cancelled()
    assert not f.done()
    f.cancel()
    assert f.cancelled()


def test_future_cancel(create_future):
    f = create_future()
    assert f.cancel()
    assert f.cancelled()
    assert f.done()
    with pytest.raises(asyncio.CancelledError):
        f.result()
    with pytest.raises(asyncio.CancelledError):
        f.exception()
    with pytest.raises(asyncio.InvalidStateError):
        f.set_result(None)
    with pytest.raises(asyncio.InvalidStateError):
        f.set_exception(None)
    assert not f.cancel()


def test_future_result(create_future):
    f = create_future()
    with pytest.raises(asyncio.InvalidStateError):
        f.result()

    f.set_result(42)
    assert not f.cancelled()
    assert f.done()
    assert f.result() == 42
    assert f.exception() is None
    with pytest.raises(asyncio.InvalidStateError):
        f.set_result(None)
    with pytest.raises(asyncio.InvalidStateError):
        f.set_exception(None)
    assert not f.cancel()


def test_future_exception(create_future):
    exc = RuntimeError()
    f = create_future()
    with pytest.raises(asyncio.InvalidStateError):
        f.exception()

    if sys.version_info[:3] > (3, 5, 1):
        # StopIteration cannot be raised into a Future - CPython issue26221
        with pytest.raises(TypeError) as excinfo:
            f.set_exception(StopIteration)

        excinfo.match("StopIteration .* cannot be raised")

    f.set_exception(exc)
    assert not f.cancelled()
    assert f.done()
    with pytest.raises(RuntimeError):
        f.result()
    assert f.exception() == exc
    with pytest.raises(asyncio.InvalidStateError):
        f.set_result(None)
    with pytest.raises(asyncio.InvalidStateError):
        f.set_exception(None)
    assert not f.cancel()


def test_future_exception_class(create_future):
    f = create_future()
    f.set_exception(RuntimeError)
    assert isinstance(f.exception(), RuntimeError)


def test_future_yield_from_twice(create_future):
    f = create_future()

    def fixture():
        yield 'A'
        x = yield from f
        yield 'B', x
        y = yield from f
        yield 'C', y

    g = fixture()
    assert next(g) == 'A'  # yield 'A'.
    assert next(g) == f  # First yield from f.
    f.set_result(42)
    assert next(g) == ('B', 42)  # yield 'B', x.
    # The second "yield from f" does not yield f.
    assert next(g) == ('C', 42)  # yield 'C', y.


@pytest.mark.skip
def test_future_repr(loop, match):
    loop.set_debug(True)
    f_pending_debug = loop.create_future()
    frame = f_pending_debug._source_traceback[-1]
    assert repr(f_pending_debug) == \
        '<Future pending created at %s:%s>' % (frame[0], frame[1])
    f_pending_debug.cancel()

    loop.set_debug(False)
    f_pending = loop.create_future()
    assert repr(f_pending) == '<Future pending>'
    f_pending.cancel()

    f_cancelled = loop.create_future()
    f_cancelled.cancel()
    assert repr(f_cancelled) == '<Future cancelled>'

    f_result = loop.create_future()
    f_result.set_result(4)
    assert repr(f_result) == '<Future finished result=4>'
    assert f_result.result() == 4

    exc = RuntimeError()
    f_exception = loop.create_future()
    f_exception.set_exception(exc)
    assert repr(f_exception) == '<Future finished exception=RuntimeError()>'
    assert f_exception.exception() is exc

    def func_repr(func):
        filename, lineno = test_utils.get_function_source(func)
        text = '%s() at %s:%s' % (func.__qualname__, filename, lineno)
        return re.escape(text)

    f_one_callbacks = loop.create_future()
    f_one_callbacks.add_done_callback(_fakefunc)
    fake_repr = func_repr(_fakefunc)
    assert match(repr(f_one_callbacks),
                 r'<Future pending cb=\[%s\]>' % fake_repr)
    f_one_callbacks.cancel()
    assert repr(f_one_callbacks) == '<Future cancelled>'

    f_two_callbacks = loop.create_future()
    f_two_callbacks.add_done_callback(first_cb)
    f_two_callbacks.add_done_callback(last_cb)
    first_repr = func_repr(first_cb)
    last_repr = func_repr(last_cb)
    assert match(repr(f_two_callbacks),
                 r'<Future pending cb=\[%s, %s\]>' % (first_repr, last_repr))

    f_many_callbacks = loop.create_future()
    f_many_callbacks.add_done_callback(first_cb)
    for i in range(8):
        f_many_callbacks.add_done_callback(_fakefunc)
        f_many_callbacks.add_done_callback(last_cb)
        cb_regex = r'%s, <8 more>, %s' % (first_repr, last_repr)
        assert match(repr(f_many_callbacks),
                     r'<Future pending cb=\[%s\]>' % cb_regex)
        f_many_callbacks.cancel()
        assert repr(f_many_callbacks) == '<Future cancelled>'


@pytest.mark.skipif(sys.version_info[:3] < (3, 5, 1),
                    reason='old python version')
def test_future_copy_state(create_future):
    from asyncio.futures import _copy_future_state

    f = create_future()
    f.set_result(10)

    newf = create_future()
    _copy_future_state(f, newf)
    assert newf.done()
    assert newf.result() == 10

    f_exception = create_future()
    f_exception.set_exception(RuntimeError())

    newf_exception = create_future()
    _copy_future_state(f_exception, newf_exception)
    assert newf_exception.done()

    with pytest.raises(RuntimeError):
        newf_exception.result()

    f_cancelled = create_future()
    f_cancelled.cancel()

    newf_cancelled = create_future()
    _copy_future_state(f_cancelled, newf_cancelled)
    assert newf_cancelled.cancelled()


def test_future_tb_logger_abandoned(create_future):
    with mock.patch('asyncio.base_events.logger') as m_log:
        fut = create_future()
        del fut
        assert not m_log.error.called


def test_future_tb_logger_result_unretrieved(create_future):
    with mock.patch('asyncio.base_events.logger') as m_log:
        fut = create_future()
        fut.set_result(42)
        del fut
        assert not m_log.error.called


def test_future_wrap_future(loop):
    def run(arg):
        return (arg, threading.get_ident())

    ex = concurrent.futures.ThreadPoolExecutor(1)
    f1 = ex.submit(run, 'oi')
    f2 = asyncio.wrap_future(f1, loop=loop)
    res, ident = loop.run_until_complete(f2)
    assert asyncio.isfuture(f2)
    assert res == 'oi'
    assert ident != threading.get_ident()


def test_future_wrap_future_future(create_future):
    f1 = create_future()
    f2 = asyncio.wrap_future(f1)
    assert f1 is f2


def test_future_wrap_future_use_global_loop(loop):
    with mock.patch('asyncio.futures.events') as events:
        events.get_event_loop = lambda: loop

        def run(arg):
            return (arg, threading.get_ident())

        ex = concurrent.futures.ThreadPoolExecutor(1)
        f1 = ex.submit(run, 'oi')
        f2 = asyncio.wrap_future(f1)
        assert loop is f2._loop


def test_future_wrap_future_cancel(loop, run_briefly):
    f1 = concurrent.futures.Future()
    f2 = asyncio.wrap_future(f1, loop=loop)
    f2.cancel()
    run_briefly(loop)
    assert f1.cancelled()
    assert f2.cancelled()


def test_future_wrap_future_cancel2(loop, run_briefly):
    f1 = concurrent.futures.Future()
    f2 = asyncio.wrap_future(f1, loop=loop)
    f1.set_result(42)
    f2.cancel()
    run_briefly(loop)
    assert not f1.cancelled()
    assert f1.result() == 42
    assert f2.cancelled()


@pytest.mark.skip
def test_future_source_traceback(loop):
    loop.set_debug(True)

    future = loop.create_future()
    lineno = sys._getframe().f_lineno - 1
    assert isinstance(future._source_traceback, list)
    assert future._source_traceback[-1][:3] == (
        __file__, lineno, 'test_future_source_traceback')


@pytest.mark.parametrize('debug', [True, False])
def check_future_exception_never_retrieved(loop, debug, run_briefly):
    last_ctx = None

    def handler(loop, context):
        nonlocal last_ctx
        last_ctx = context

    loop.set_debug(debug)
    loop.set_exception_handler(handler)

    def memory_error():
        try:
            raise MemoryError()
        except BaseException as exc:
            return exc

    exc = memory_error()

    future = loop.create_future()
    if debug:
        # source_traceback = future._source_traceback
        future.set_exception(exc)
        future = None
        support.gc_collect()
        run_briefly(loop)

    assert last_ctx is not None

    assert last_ctx['exception'] is exc
    assert last_ctx['message'] == 'Future exception was never retrieved'

    if debug:
        tb = last_ctx['source_traceback']
        assert tb[-2].name == 'check_future_exception_never_retrieved'


def test_future_wrap_future2(loop):
    from uvloop.loop import _wrap_future

    def run(arg):
        return (arg, threading.get_ident())

    ex = concurrent.futures.ThreadPoolExecutor(1)
    f1 = ex.submit(run, 'oi')
    f2 = _wrap_future(f1, loop=loop)
    res, ident = loop.run_until_complete(f2)
    assert asyncio.isfuture(f2)
    assert res == 'oi'
    assert ident != threading.get_ident()


def test_future_wrap_future_future3(create_future):
    from uvloop.loop import _wrap_future
    f1 = create_future()
    f2 = _wrap_future(f1)
    assert f1 is f2


def test_future_wrap_future_cancel4(loop, run_briefly):
    from uvloop.loop import _wrap_future
    f1 = concurrent.futures.Future()
    f2 = _wrap_future(f1, loop=loop)
    f2.cancel()
    run_briefly(loop)
    assert f1.cancelled()
    assert f2.cancelled()


def test_future_wrap_future_cancel5(loop, run_briefly):
    from uvloop.loop import _wrap_future
    f1 = concurrent.futures.Future()
    f2 = _wrap_future(f1, loop=loop)
    f1.set_result(42)
    f2.cancel()
    run_briefly(loop)
    assert not f1.cancelled()
    assert f1.result() == 42
    assert f2.cancelled()


def test_future_callbacks_invoked_on_set_result(
        loop, create_future, make_callback, run_briefly):
    bag = []
    f = create_future()
    f.add_done_callback(make_callback(bag, 42))
    f.add_done_callback(make_callback(bag, 17))

    assert bag == []
    f.set_result('foo')

    run_briefly(loop)

    assert bag == [42, 17]
    assert f.result() == 'foo'


def test_future_callbacks_invoked_on_set_exception(
        loop, create_future, make_callback, run_briefly):
    bag = []
    f = create_future()
    f.add_done_callback(make_callback(bag, 100))

    assert bag == []

    exc = RuntimeError()
    f.set_exception(exc)

    run_briefly(loop)

    assert bag == [100]
    assert f.exception() == exc


def test_future_remove_done_callback(
        loop, create_future, make_callback, run_briefly):
    bag = []
    f = create_future()
    cb1 = make_callback(bag, 1)
    cb2 = make_callback(bag, 2)
    cb3 = make_callback(bag, 3)

    # Add one cb1 and one cb2.
    f.add_done_callback(cb1)
    f.add_done_callback(cb2)

    # One instance of cb2 removed. Now there's only one cb1.
    assert f.remove_done_callback(cb2) == 1

    # Never had any cb3 in there.
    assert f.remove_done_callback(cb3) == 0

    # After this there will be 6 instances of cb1 and one of cb2.
    f.add_done_callback(cb2)
    for i in range(5):
        f.add_done_callback(cb1)

    # Remove all instances of cb1. One cb2 remains.
    assert f.remove_done_callback(cb1) == 6

    assert bag == []
    f.set_result('foo')

    run_briefly(loop)

    assert bag == [2]
    assert f.result() == 'foo'

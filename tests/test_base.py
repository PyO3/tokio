# Copied from the uvloop project.  If you add a new unittest here,
# please consider contributing it to the uvloop project.
#
# Portions copyright (c) 2015-present MagicStack Inc.  http://magic.io

import asyncio
import logging
import os
import threading
import time
import weakref
from unittest import mock

import pytest
import uvloop


def test_close(loop):
    assert not loop.is_closed()
    loop.close()
    assert loop.is_closed()

    # it should be possible to call close() more than once
    loop.close()
    loop.close()

    # operation blocked when the loop is closed
    f = asyncio.Future(loop=loop)

    with pytest.raises(RuntimeError):
        loop.run_forever()

    with pytest.raises(RuntimeError):
        loop.run_until_complete(f)


@pytest.mark.skip
def test_handle_weakref(loop):
    wd = weakref.WeakValueDictionary()
    h = loop.call_soon(lambda: None)
    wd['h'] = h  # Would fail without __weakref__ slot.


def test_call_soon(loop):
    calls = []

    def cb(inc):
        calls.append(inc)
        loop.stop()

    loop.call_soon(cb, 10)

    h = loop.call_soon(cb, 100)
    # self.assertIn('.cb', repr(h))
    h.cancel()
    # self.assertIn('cancelled', repr(h))

    loop.call_soon(cb, 1)
    loop.run_forever()

    assert calls == [10, 1]


def test_call_soon_base_exc(loop):
    def cb():
        raise KeyboardInterrupt()

    loop.call_soon(cb)

    with pytest.raises(KeyboardInterrupt):
        loop.run_forever()

    assert not loop.is_closed()


@pytest.mark.parametrize('debug', [True, False])
@pytest.mark.parametrize(
    'name, meth',
    [('call_soon', lambda loop, *args: loop.call_soon(*args)),
     ('call_later', lambda loop, *args: loop.call_later(0.01, *args))])
def test_calls_debug_reporting(loop, debug, name, meth):
    context = None

    def handler(loop, ctx):
        nonlocal context
        context = ctx

    loop.set_debug(debug)
    loop.set_exception_handler(handler)

    def cb():
        1 / 0

    meth(loop, cb)
    assert context is None
    loop.run_until_complete(asyncio.sleep(0.05, loop=loop))

    assert type(context['exception']) is ZeroDivisionError
    assert context['message'].startswith('Exception in callback')

    if debug:
        tb = context['source_traceback']
        assert tb[-2].name == 'test_calls_debug_reporting'
    else:
        assert 'source_traceback' not in context

    del context


def test_now_update(loop):
    async def run():
        st = loop.time()
        time.sleep(0.05)
        return loop.time() - st

    delta = loop.run_until_complete(run())
    assert delta > 0.049 and delta < 0.6


def test_call_later_1(loop):
    calls = []

    def cb(inc=10, stop=False):
        calls.append(inc)
        assert loop.is_running()
        if stop:
            loop.call_soon(loop.stop)

    loop.call_later(0.05, cb)

    # canceled right away
    h = loop.call_later(0.05, cb, 100, True)
    # assert '.cb' in repr(h)
    h.cancel()
    # assert 'cancelled' in repr(h)

    loop.call_later(0.05, cb, 1, True)
    loop.call_later(1000, cb, 1000)  # shouldn't be called

    started = time.monotonic()
    loop.run_forever()
    finished = time.monotonic()

    assert calls == [10, 1]
    assert not loop.is_running()

    assert finished - started < 0.1
    assert finished - started > 0.04


def test_call_later_2(loop):
    # Test that loop.call_later triggers an update of
    # libuv cached time.

    async def main():
        await asyncio.sleep(0.001, loop=loop)
        time.sleep(0.01)
        await asyncio.sleep(0.01, loop=loop)

    started = time.monotonic()
    loop.run_until_complete(main())
    delta = time.monotonic() - started
    assert delta > 0.019


def test_call_later_negative(loop):
    calls = []

    def cb(arg):
        calls.append(arg)
        loop.stop()

    loop.call_later(-1, cb, 'a')
    loop.run_forever()
    assert calls == ['a']


@pytest.mark.skipif(os.environ.get('TRAVIS_OS_NAME') is not None,
                    reason='time is not monotonic on Travis')
def test_call_at(loop):
    i = 0

    def cb(inc):
        nonlocal i
        i += inc
        loop.stop()

    at = loop.time() + 0.05

    loop.call_at(at, cb, 100).cancel()
    loop.call_at(at, cb, 10)

    started = time.monotonic()
    loop.run_forever()
    finished = time.monotonic()

    assert i == 10

    assert finished - started < 0.07
    assert finished - started > 0.045


def test_check_thread(loop, other_loop):
    def check_thread(loop, debug):
        def cb():
            pass

        loop.set_debug(debug)
        if debug:
            msg = ("Non-thread-safe operation invoked on an "
                   "event loop other than the current one")
            with pytest.raises(RuntimeError) as exc:
                loop.call_soon(cb)
            exc.match(msg)

            with pytest.raises(RuntimeError) as exc:
                loop.call_later(60, cb)
            exc.match(msg)

            with pytest.raises(RuntimeError) as exc:
                loop.call_at(loop.time() + 60, cb)
            exc.match(msg)
        else:
            loop.call_soon(cb)
            loop.call_later(60, cb)
            loop.call_at(loop.time() + 60, cb)

    def check_in_thread(loop, event, debug, create_loop, fut):
        # wait until the event loop is running
        event.wait()

        try:
            if create_loop:
                try:
                    asyncio.set_event_loop(other_loop)
                    check_thread(loop, debug)
                finally:
                    asyncio.set_event_loop(None)
            else:
                check_thread(loop, debug)
        except Exception as exc:
            loop.call_soon_threadsafe(fut.set_exception, exc)
        else:
            loop.call_soon_threadsafe(fut.set_result, None)

    def test_thread(loop, debug, create_loop=False):
        event = threading.Event()
        fut = asyncio.Future(loop=loop)
        loop.call_soon(event.set)
        args = (loop, event, debug, create_loop, fut)
        thread = threading.Thread(target=check_in_thread, args=args)
        thread.start()
        loop.run_until_complete(fut)
        thread.join()

    # raise RuntimeError if the thread has no event loop
    # test_thread(loop, True)

    # check disabled if debug mode is disabled
    # test_thread(loop, False)

    # raise RuntimeError if the event loop of the thread is not the called
    # event loop
    # test_thread(loop, True, create_loop=True)

    # check disabled if debug mode is disabled
    # test_thread(loop, False, create_loop=True)


def test_run_once_in_executor_plain(loop):
    called = []

    def cb(arg):
        called.append(arg)

    async def runner():
        await loop.run_in_executor(None, cb, 'a')

    loop.run_until_complete(runner())

    assert called == ['a']


def test_set_debug(loop):
    loop.set_debug(True)
    assert loop.get_debug()
    loop.set_debug(False)
    assert not loop.get_debug()


def test_run_until_complete_type_error(loop):
    with pytest.raises(TypeError):
        loop.run_until_complete('blah')


def test_run_until_complete_loop(loop, other_loop):
    task = asyncio.Future(loop=loop)
    with pytest.raises(ValueError):
        other_loop.run_until_complete(task)


def test_run_until_complete_error(loop):
    async def foo():
        raise ValueError('aaa')

    with pytest.raises(ValueError, message='aaa'):
        loop.run_until_complete(foo())


@pytest.mark.skip(reason='tokio is not support this')
def test_debug_slow_callbacks(loop):
    logger = logging.getLogger('asyncio')
    loop.set_debug(True)
    loop.slow_callback_duration = 0.2
    loop.call_soon(lambda: time.sleep(0.3))

    with mock.patch.object(logger, 'warning') as log:
        loop.run_until_complete(asyncio.sleep(0, loop=loop))

    assert log.call_count == 1

    # format message
    msg = log.call_args[0][0] % log.call_args[0][1:]

    assert 'Executing <Handle' in msg
    assert 'test_debug_slow_callbacks' in msg


@pytest.mark.skip(reason='tokio is not support this')
def test_debug_slow_timer_callbacks(loop):
    logger = logging.getLogger('asyncio')
    loop.set_debug(True)
    loop.slow_callback_duration = 0.2
    loop.call_later(0.01, lambda: time.sleep(0.3))

    with mock.patch.object(logger, 'warning') as log:
        loop.run_until_complete(asyncio.sleep(0.02, loop=loop))

    assert log.call_count == 1

    # format message
    # msg = log.call_args[0][0] % log.call_args[0][1:]

    # self.assertIn('Executing <Handle', msg)
    # self.assertIn('test_debug_slow_callbacks', msg)


@pytest.mark.skip(reason='tokio is not support this')
def test_default_exc_handler_callback(loop, mock_pattern):
    loop._process_events = mock.Mock()

    def zero_error(fut):
        fut.set_result(True)
        1 / 0

    logger = logging.getLogger('asyncio')

    # Test call_soon (events.Handle)
    with mock.patch.object(logger, 'error') as log:
        fut = asyncio.Future(loop=loop)
        loop.call_soon(zero_error, fut)
        fut.add_done_callback(lambda fut: loop.stop())
        loop.run_forever()
        log.assert_called_with(
            mock_pattern('Exception in callback.*zero'), exc_info=mock.ANY)

    # Test call_later (events.TimerHandle)
    with mock.patch.object(logger, 'error') as log:
        fut = asyncio.Future(loop=loop)
        loop.call_later(0.01, zero_error, fut)
        fut.add_done_callback(lambda fut: loop.stop())
        loop.run_forever()
        log.assert_called_with(
            mock_pattern('Exception in callback.*zero'), exc_info=mock.ANY)


@pytest.mark.skip(reason='need tokio logging decision')
def test_set_exc_handler_custom(loop, mock_pattern, match):
    logger = logging.getLogger('asyncio')

    def run_loop():
        def zero_error():
            loop.stop()
            1 / 0
        loop.call_soon(zero_error)
        loop.run_forever()

    errors = []

    def handler(loop, exc):
        errors.append(exc)

    loop.set_debug(True)

    if hasattr(loop, 'get_exception_handler'):
        # Available since Python 3.5.2
        assert loop.get_exception_handler() is None

    loop.set_exception_handler(handler)
    if hasattr(loop, 'get_exception_handler'):
        assert loop.get_exception_handler() is handler

    run_loop()
    assert len(errors) == 1
    assert match(errors[-1]['message'], 'Exception in callback.*zero_error')

    loop.set_exception_handler(None)
    with mock.patch.object(logger, 'error') as log:
        run_loop()
        log.assert_called_with(
            mock_pattern('Exception in callback.*zero'), exc_info=mock.ANY)

    assert len(errors) == 1


@pytest.mark.skip(reason='need tokio logging decision')
def test_set_exc_handler_broken(loop, mock_pattern):
    logger = logging.getLogger('asyncio')

    def run_loop():
        def zero_error():
            loop.stop()
            1 / 0
        loop.call_soon(zero_error)
        loop.run_forever()

    def handler(loop, context):
        raise AttributeError('spam')

    loop._process_events = mock.Mock()

    loop.set_exception_handler(handler)

    with mock.patch.object(logger, 'error') as log:
        run_loop()
        log.assert_called_with(
            mock_pattern('Unhandled error in exception handler'),
            exc_info=mock.ANY)


@pytest.mark.skip(reason='need impl')
def test_default_exc_handler_broken(loop, mock_pattern):
    logger = logging.getLogger('asyncio')
    _context = None

    class Loop(uvloop.Loop):

        _selector = mock.Mock()
        _process_events = mock.Mock()

        def default_exception_handler(self, context):
            nonlocal _context
            _context = context
            # Simulates custom buggy "default_exception_handler"
            raise ValueError('spam')

    loop = Loop()
    # self.addCleanup(loop.close)
    asyncio.set_event_loop(loop)

    def run_loop():
        def zero_error():
            loop.stop()
            1 / 0
        loop.call_soon(zero_error)
        loop.run_forever()

    with mock.patch.object(logger, 'error') as log:
        run_loop()
        log.assert_called_with(
            'Exception in default exception handler',
            exc_info=True)

    def custom_handler(loop, context):
        raise ValueError('ham')

    _context = None
    loop.set_exception_handler(custom_handler)
    with mock.patch.object(logger, 'error') as log:
        run_loop()
        log.assert_called_with(
            mock_pattern('Exception in default exception.*'
                         'while handling.*in custom'),
            exc_info=True)

        # Check that original context was passed to default
        # exception handler.
        assert 'context' in _context
        assert (type(_context['context']['exception']) is
                ZeroDivisionError)


@pytest.mark.skip(reason='need impl')
def test_set_task_factory_invalid(loop):
    with pytest.raises(
            TypeError, message='task factory must be a callable or None'):
        loop.set_task_factory(1)

    assert loop.get_task_factory() is None


@pytest.mark.skip(reason='need impl')
def test_set_task_factory(loop):
    # loop._process_events = mock.Mock()

    class MyTask(asyncio.Task):
        pass

    @asyncio.coroutine
    def coro():
        pass

    def factory(loop, coro):
        return MyTask(coro, loop=loop)

    assert loop.get_task_factory() is None
    loop.set_task_factory(factory)
    assert loop.get_task_factory() is factory

    task = loop.create_task(coro())
    assert isinstance(task, MyTask)
    loop.run_until_complete(task)

    loop.set_task_factory(None)
    assert loop.get_task_factory() is None

    task = loop.create_task(coro())
    assert isinstance(task, asyncio.Task)
    assert not isinstance(task, MyTask)
    loop.run_until_complete(task)

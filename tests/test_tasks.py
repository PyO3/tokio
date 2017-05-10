# Copied from the uvloop project.  If you add a new unittest here,
# please consider contributing it to the uvloop project.
#
# Portions copyright (c) 2015-present MagicStack Inc.  http://magic.io

import asyncio
from asyncio import test_utils

import pytest


class Dummy:

    def __repr__(self):
        return '<Dummy>'

    def __call__(self, *args):
        pass


def format_coroutine(qualname, state, src, source_traceback, generator=False):
    if generator:
        state = '%s' % state
    else:
        state = '%s, defined' % state
    if source_traceback is not None:
        frame = source_traceback[0]
        return ('coro=<%s() %s at %s> created at %s:%s'
                % (qualname, state, src, frame[0], frame[1]))
    else:
        return 'coro=<%s() %s at %s>' % (qualname, state, src)


def test_task_repr(loop):
    loop.set_debug(False)

    @asyncio.coroutine
    def notmuch():
        yield from []
        return 'abc'

    # test coroutine function
    assert notmuch.__name__ == 'notmuch'
    assert notmuch.__qualname__ == 'test_task_repr.<locals>.notmuch'
    assert notmuch.__module__ == __name__

    filename, lineno = test_utils.get_function_source(notmuch)
    src = "%s:%s" % (filename, lineno)

    # test coroutine object
    gen = notmuch()
    coro_qualname = 'test_task_repr.<locals>.notmuch'
    assert gen.__name__ == 'notmuch'
    assert gen.__qualname__ == coro_qualname

    # test pending Task
    t = asyncio.Task(gen, loop=loop)
    t.add_done_callback(Dummy())

    coro = format_coroutine(coro_qualname, 'running', src,
                            t._source_traceback, generator=True)
    assert repr(t) == '<Task pending %s cb=[<Dummy>()]>' % coro

    # test canceling Task
    t.cancel()  # Does not take immediate effect!
    assert repr(t) == '<Task cancelling %s cb=[<Dummy>()]>' % coro

    # test canceled Task
    with pytest.raises(asyncio.CancelledError):
        loop.run_until_complete(t)

    coro = format_coroutine(coro_qualname, 'done', src, t._source_traceback)
    assert repr(t) == '<Task cancelled %s>' % coro

    # test finished Task
    t = asyncio.Task(notmuch(), loop=loop)
    loop.run_until_complete(t)
    coro = format_coroutine(coro_qualname, 'done', src, t._source_traceback)
    assert repr(t) == "<Task finished %s result='abc'>" % coro


def test_task_basics(loop):
    @asyncio.coroutine
    def outer():
        a = yield from inner1()
        b = yield from inner2()
        return a + b

    @asyncio.coroutine
    def inner1():
        return 42

    @asyncio.coroutine
    def inner2():
        return 1000

    t = outer()
    assert loop.run_until_complete(t) == 1042


def test_task_cancel_yield(loop, create_task):
    @asyncio.coroutine
    def task():
        while True:
            yield
        return 12

    t = create_task(task())
    test_utils.run_briefly(loop)  # start coro
    t.cancel()
    with pytest.raises(asyncio.CancelledError):
        loop.run_until_complete(t)

    assert t.done()
    assert t.cancelled()
    assert not t.cancel()


def test_task_cancel_inner_future(loop, create_future, create_task):
    f = create_future()

    @asyncio.coroutine
    def task():
        yield from f
        return 12

    t = create_task(task())
    test_utils.run_briefly(loop)  # start task
    f.cancel()
    with pytest.raises(asyncio.CancelledError):
        loop.run_until_complete(t)
        assert f.cancelled()
        assert t.cancelled()


def test_task_cancel_both_task_and_inner_future(
        loop, create_future, create_task):
    f = create_future()

    @asyncio.coroutine
    def task():
        yield from f
        return 12

    t = create_task(task())
    # assert asyncio.Task.all_tasks(loop=loop) == {t}
    test_utils.run_briefly(loop)

    f.cancel()
    t.cancel()

    with pytest.raises(asyncio.CancelledError):
        loop.run_until_complete(t)

    assert t.done()
    assert f.cancelled()
    assert t.cancelled()


def test_task_cancel_task_catching(loop, create_future, create_task):
    fut1 = create_future()
    fut2 = create_future()

    @asyncio.coroutine
    def task():
        yield from fut1
        try:
            yield from fut2
        except asyncio.CancelledError:
            return 42

    t = create_task(task())
    test_utils.run_briefly(loop)
    assert t._fut_waiter is fut1  # White-box test.
    fut1.set_result(None)
    test_utils.run_briefly(loop)
    assert t._fut_waiter is fut2  # White-box test.
    t.cancel()
    assert fut2.cancelled()
    res = loop.run_until_complete(t)
    assert res == 42
    assert not t.cancelled()


def test_task_cancel_task_ignoring(loop, create_future, create_task):
    fut1 = create_future()
    fut2 = create_future()
    fut3 = create_future()

    @asyncio.coroutine
    def task():
        yield from fut1
        try:
            yield from fut2
        except asyncio.CancelledError:
            pass
        res = yield from fut3
        return res

    t = create_task(task())
    test_utils.run_briefly(loop)
    assert t._fut_waiter is fut1  # White-box test.
    fut1.set_result(None)
    test_utils.run_briefly(loop)
    assert t._fut_waiter is fut2  # White-box test.
    t.cancel()
    assert fut2.cancelled()
    test_utils.run_briefly(loop)
    assert t._fut_waiter is fut3  # White-box test.
    fut3.set_result(42)
    res = loop.run_until_complete(t)
    assert res == 42
    assert not fut3.cancelled()
    assert not t.cancelled()


def test_task_cancel_current_task(loop, create_future, create_task):
    @asyncio.coroutine
    def task():
        t.cancel()
        assert t._must_cancel  # White-box test.
        # The sleep should be canceled immediately.
        yield from asyncio.sleep(100, loop=loop)
        return 12

    t = create_task(task())
    with pytest.raises(asyncio.CancelledError):
        loop.run_until_complete(t)

    assert t.done()
    assert not t._must_cancel  # White-box test.
    assert not t.cancel()


def test_task_step_with_baseexception(loop, create_future, create_task):
    @asyncio.coroutine
    def notmutch():
        raise BaseException()

    task = create_task(notmutch())
    with pytest.raises(BaseException):
        loop.run_until_complete(task)

    assert task.done()
    assert isinstance(task.exception(), BaseException)


def test_task_step_result_future(loop, create_future, create_task):
    # If coroutine returns future, task waits on this future.

    class Fut(asyncio.Future):
        def __init__(self, *args, **kwds):
            self.cb_added = False
            super().__init__(*args, **kwds)

        def add_done_callback(self, fn):
            self.cb_added = True
            super().add_done_callback(fn)

    fut = Fut(loop=loop)
    result = None

    @asyncio.coroutine
    def wait_for_future():
        nonlocal result
        result = yield from fut

    t = create_task(wait_for_future())
    test_utils.run_briefly(loop)
    assert fut.cb_added

    res = object()
    fut.set_result(res)
    test_utils.run_briefly(loop)
    assert res is result
    assert t.done()
    assert t.result() is None


def test_task_step_result(loop):
    @asyncio.coroutine
    def notmuch():
        yield None
        yield 1
        return 'ko'

    with pytest.raises(RuntimeError):
        loop.run_until_complete(notmuch())


def test_task_yield_vs_yield_from(loop):
    fut = asyncio.Future(loop=loop)

    @asyncio.coroutine
    def wait_for_future():
        yield fut

    task = wait_for_future()
    with pytest.raises(RuntimeError):
        loop.run_until_complete(task)

    assert not fut.done()


@pytest.mark.skip
def test_task_current_task(loop, create_future, create_task):
    assert asyncio.Task.current_task(loop=loop) is None

    @asyncio.coroutine
    def coro(loop):
        assert asyncio.Task.current_task(loop=loop) is task

    task = create_task(coro(loop))
    loop.run_until_complete(task)
    assert asyncio.Task.current_task(loop=loop) is None


def test_task_current_task_with_interleaving_tasks(
        loop, create_future, create_task):
    assert asyncio.Task.current_task(loop=loop) is None

    fut1 = create_future()
    fut2 = create_future()

    @asyncio.coroutine
    def coro1(loop):
        assert asyncio.Task.current_task(loop=loop) is task1
        yield from fut1
        assert asyncio.Task.current_task(loop=loop) is task1
        fut2.set_result(True)

    @asyncio.coroutine
    def coro2(loop):
        assert asyncio.Task.current_task(loop=loop) is task2
        fut1.set_result(True)
        yield from fut2
        assert asyncio.Task.current_task(loop=loop) is task2

    task1 = create_task(coro1(loop))
    task2 = create_task(coro2(loop))

    loop.run_until_complete(asyncio.wait((task1, task2), loop=loop))
    assert asyncio.Task.current_task(loop=loop) is None


def test_task_yield_future_passes_cancel(
        loop, create_future, create_task):
    # Canceling outer() cancels inner() cancels waiter.
    proof = 0
    waiter = create_future()

    @asyncio.coroutine
    def inner():
        nonlocal proof
        try:
            yield from waiter
        except asyncio.CancelledError:
            proof += 1
            raise
        else:
            assert False, 'got past sleep() in inner()'

    @asyncio.coroutine
    def outer():
        nonlocal proof
        try:
            yield from inner()
        except asyncio.CancelledError:
            proof += 100  # Expect this path.
        else:
            proof += 10

    f = asyncio.ensure_future(outer(), loop=loop)
    test_utils.run_briefly(loop)
    f.cancel()
    loop.run_until_complete(f)
    assert proof == 101
    assert waiter.cancelled()


def test_task_yield_wait_does_not_shield_cancel(
        loop, create_future, create_task):
    # Canceling outer() makes wait() return early, leaves inner() running.
    proof = 0
    waiter = create_future()

    @asyncio.coroutine
    def inner():
        nonlocal proof
        yield from waiter
        proof += 1

    @asyncio.coroutine
    def outer():
        nonlocal proof
        d, p = yield from asyncio.wait([inner()], loop=loop)
        proof += 100

    f = asyncio.ensure_future(outer(), loop=loop)
    test_utils.run_briefly(loop)
    f.cancel()

    with pytest.raises(asyncio.CancelledError):
        loop.run_until_complete(f)

    waiter.set_result(None)
    test_utils.run_briefly(loop)
    assert proof == 1

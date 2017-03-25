import asyncio
from asyncio import tasks


@asyncio.coroutine
def sleep(delay, result=None, *, loop=None):
    """Coroutine that completes after a given time (in seconds)."""
    if delay == 0:
        yield
        return result

    if loop is None:
        loop = events.get_event_loop()

    future = loop.create_future()
    h = loop.call_later(delay,
                        futures._set_result_unless_cancelled,
                        future, result)
    try:
        return (yield from future)
    finally:
        h.cancel()


@asyncio.coroutine
def ensure_future(coro_or_future, *, loop=None):
    """Wrap a coroutine or an awaitable in a future.

    If the argument is a Future, it is returned directly.
    """
    if futures.isfuture(coro_or_future):
        if loop is not None and loop is not coro_or_future._loop:
            raise ValueError('loop argument must agree with Future')
        return coro_or_future
    elif coroutines.iscoroutine(coro_or_future):
        if loop is None:
            loop = events.get_event_loop()
        task = loop.create_task(coro_or_future)
        source_traceback = getattr(task, '_source_traceback', None)
        if source_traceback:
            del source_traceback[-1]
        return task
    elif compat.PY35 and inspect.isawaitable(coro_or_future):
        return ensure_future(_wrap_awaitable(coro_or_future), loop=loop)
    else:
        raise TypeError('A Future, a coroutine or an awaitable is required')


def patch_asyncio():
    asyncio.sleep.__code__ = sleep.__code__
    asyncio.ensure_future.__code__ = ensure_future.__code__
    tasks.ensure_future.__code__ = ensure_future.__code__

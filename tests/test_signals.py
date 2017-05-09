import signal

import pytest


def test_signals_invalid_signal(loop):
    with pytest.raises(RuntimeError) as excinfo:
        loop.add_signal_handler(signal.SIGKILL, lambda *a: None)

    excinfo.match('sig {} cannot be caught'.format(signal.SIGKILL))


def test_signals_coro_callback(loop):
    async def coro(): pass

    with pytest.raises(TypeError) as excinfo:
        loop.add_signal_handler(signal.SIGHUP, coro)

    excinfo.match('coroutines cannot be used')

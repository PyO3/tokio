# Copied from the uvloop project.  If you add a new unittest here,
# please consider contributing it to the uvloop project.
#
# Portions copyright (c) 2015-present MagicStack Inc.  http://magic.io

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

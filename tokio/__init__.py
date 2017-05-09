import os
os.environ['RUST_LOG'] = 'async_tokio=debug'  # noqa

import gc
from asyncio.events import AbstractEventLoop, BaseDefaultEventLoopPolicy

gc.disable()  # noqa

from . import _tokio

gc.enable()  # noqa

__all__ = ('new_event_loop', 'TokioLoopPolicy')


def new_event_loop():
    return _tokio.new_event_loop()


class TokioLoopPolicy(BaseDefaultEventLoopPolicy):
    """Event loop policy."""

    def _loop_factory(self):
        return new_event_loop()

    def set_event_loop(self, loop):
        """Set the event loop."""
        self._local._set_called = True
        assert loop is None or isinstance(
            loop, (AbstractEventLoop, _tokio.TokioEventLoop))
        self._local._loop = loop

import os
os.environ['RUST_LOG'] = 'async_tokio=debug'  # noqa

from asyncio.events import BaseDefaultEventLoopPolicy as __BasePolicy

from . import _tokio

__all__ = ('new_event_loop', 'EventLoopPolicy')


def new_event_loop():
    return _tokio.new_event_loop()


class EventLoopPolicy(__BasePolicy):
    """Event loop policy."""

    def _loop_factory(self):
        return new_event_loop()

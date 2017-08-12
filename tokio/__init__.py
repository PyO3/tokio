__version__ = '0.3.0.dev0'

# import os
# os.environ['RUST_LOG'] = 'async_tokio=debug'  # noqa

from asyncio.events import AbstractEventLoop
from asyncio.unix_events import DefaultEventLoopPolicy

from . import _tokio

__all__ = ('new_event_loop', 'Loop', 'EventLoopPolicy')


class Loop(_tokio.TokioEventLoop, AbstractEventLoop):
    pass


def new_event_loop():
    return Loop()


class EventLoopPolicy(DefaultEventLoopPolicy):
    """Event loop policy."""

    def _loop_factory(self):
        return Loop()

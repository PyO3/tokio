import asyncio  # noqa
from . import _tokio

__all__ = ('new_event_loop', 'EventLoopPolicy')


def new_event_loop():
    return _tokio.new_event_loop()


class EventLoopPolicy:
    """Event loop policy."""

    def _loop_factory(self):
        return new_event_loop()

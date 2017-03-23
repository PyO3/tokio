# import asyncio

from . import _ext

__all__ = ('new_event_loop', 'EventLoopPolicy')


def new_event_loop():
    return _ext.new_event_loop()


class EventLoopPolicy:
    """Event loop policy."""

    def _loop_factory(self):
        return new_event_loop()

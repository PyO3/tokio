import reprlib
from asyncio import events


def _format_callbacks(cb):
    """helper function for Future.__repr__"""
    size = len(cb)
    if not size:
        cb = ''

    def format_cb(callback):
        return events._format_callback_source(callback, ())

    if size == 1:
        cb = format_cb(cb[0])
    elif size == 2:
        cb = '{}, {}'.format(format_cb(cb[0]), format_cb(cb[1]))
    elif size > 2:
        cb = '{}, <{} more>, {}'.format(format_cb(cb[0]),
                                        size - 2,
                                        format_cb(cb[-1]))
    return 'cb=[%s]' % cb


def future_repr(name, future):
    # (Future) -> str
    """helper function for Future.__repr__"""
    info = []

    if future.done():
        info.append('finished')
        if future._exception is not None:
            info.append('exception={!r}'.format(future._exception))
        else:
            # use reprlib to limit the length of the output, especially
            # for very long strings
            result = reprlib.repr(future._result)
            info.append('result={}'.format(result))
    elif future.cancelled():
        info.append('cancelled')
    else:
        info.append('pending')

    if future._callbacks:
        info.append(_format_callbacks(future._callbacks))
    if future._source_traceback:
        frame = future._source_traceback[-1]
        info.append('created at %s:%s' % (frame[0], frame[1]))

    return '<%s %s>' % (name, ' '.join(info))

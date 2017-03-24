
try:
    from ascynio import CancelledError, TimeoutError, InvalidStateError

except ImportError:

    class Error(Exception):
        pass

    class CancelledError(Error):
        pass

    class TimeoutError(Error):
        pass

    class InvalidStateError(Error):
        pass

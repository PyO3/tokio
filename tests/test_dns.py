# Copied from the uvloop project.  If you add a new unittest here,
# please consider contributing it to the uvloop project.
#
# Portions copyright (c) 2015-present MagicStack Inc.  http://magic.io

import socket

import pytest


@pytest.mark.parametrize(
    'args', [(('example.com', 80), {}),
             (('example.com', 80), {'type': socket.SOCK_STREAM}),
             (('example.com', 80), {'flags': socket.AI_CANONNAME}),
             (('a' + '1' * 50 + '.wat', 800), {}),
             (('example.com', 80), {'family': -1}),
             (('example.com', 80), {'type': socket.SOCK_STREAM, 'family': -1}),
             (('example.com', '80'), {}),
             (('example.com', '80'), {'type': socket.SOCK_STREAM}),
             ((b'example.com', b'80'), {}),
             ((b'example.com', b'80'), {'type': socket.SOCK_STREAM}),
             ((None, 0), {}),
             ((None, 0), {'type': socket.SOCK_STREAM}),
             (('', 0), {}),
             (('', 0), {'type': socket.SOCK_STREAM}),
             ((b'', 0), {}),
             ((b'', 0), {'type': socket.SOCK_STREAM}),
             ((None, None), {}),
             ((None, None), {'type': socket.SOCK_STREAM}),
             ((b'example.com', '80'), {}),
             ((b'example.com', '80'), {'type': socket.SOCK_STREAM}),
             (('127.0.0.1', '80'), {}),
             (('127.0.0.1', '80'), {'type': socket.SOCK_STREAM}),
             ((b'127.0.0.1', b'80'), {}),
             ((b'127.0.0.1', b'80'), {'type': socket.SOCK_STREAM}),
             ((b'127.0.0.1', b'http'), {}),
             ((b'127.0.0.1', b'http'), {'type': socket.SOCK_STREAM}),
             (('127.0.0.1', 'http'), {}),
             (('127.0.0.1', 'http'), {'type': socket.SOCK_STREAM}),
             (('localhost', 'http'), {}),
             (('localhost', 'http'), {'type': socket.SOCK_STREAM}),

             ((b'localhost', 'http'), {}),
             ((b'localhost', 'http'), {'type': socket.SOCK_STREAM}),

             (('localhost', b'http'), {}),
             (('localhost', b'http'), {'type': socket.SOCK_STREAM}),

             (('::1', 80), {}),
             (('::1', 80), {'type': socket.SOCK_STREAM}),

             (('127.0.0.1', 80), {}),
             (('127.0.0.1', 80), {'type': socket.SOCK_STREAM})])
def test_getaddrinfo(loop, args):
    err = None
    try:
        a1 = socket.getaddrinfo(*args[0], **args[1])
    except socket.gaierror as ex:
        err = ex

    try:
        a2 = loop.run_until_complete(loop.getaddrinfo(*args[0], **args[1]))
    except socket.gaierror as ex:
        if err is not None:
            ex.args == err.args
        else:
            ex.__context__ = err
            raise ex
    except OSError as ex:
        ex.__context__ = err
        raise ex
    else:
        if err is not None:
            raise err

        assert a1 == a2


@pytest.mark.parametrize(
    'args', [(('127.0.0.1', 80), 0),
             (('127.0.0.1', 80, 1231231231213), 0),
             (('127.0.0.1', 80, 0, 0), 0),
             (('::1', 80), 0),
             (('localhost', 8080), 0)])
def test_getnameinfo(loop, args):
    err = None
    try:
        a1 = socket.getnameinfo(*args)
    except Exception as ex:
        err = ex

    try:
        a2 = loop.run_until_complete(loop.getnameinfo(*args))
    except Exception as ex:
        if err is not None:
            if ex.__class__ is not err.__class__:
                print(ex, err)
                assert ex.__class__ == err.__class__
                assert ex.args == err.args
        else:
            raise
    else:
        if err is not None:
            raise err

        assert a1 == a2

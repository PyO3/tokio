from __future__ import print_function
import asyncio
import traceback
import os
from aiohttp import web

import tokio


class ProtocolWrapper:

    def __init__(self, protocol):
        self._loop = asyncio.get_event_loop()
        self.__actions = []
        self.__scheduled = False

        self.__protocol = protocol
        protocol.connection_made = self._wrap_method(protocol.factory.connection_made)
        protocol.connection_lost = self._wrap_method(protocol.factory.connection_lost, True)
        protocol.data_received = self._wrap_method(protocol.factory.data_received)

    def _exec(self):
        self.__scheduled = False
        actions = self.__actions
        self.__actions = []

        for (meth, proto, args) in actions:
            try:
                meth(proto, *args)
            except:
                pass

    def __call__(self):
        # create new connection
        proto = self.__protocol()
        return proto

    def _wrap_method(self, meth, remove=False):
        # @functools.wraps
        def wrapper(proto, *args, **kwargs):
            self.__actions.append((meth, proto, args))
            if not self.__scheduled:
                self.__scheduled = True
                self._loop.call_soon_threadsafe(self._exec)

        return wrapper


def test_web():
    print(os.getpid())
    name = 'test_web'
    print(dir(tokio))
    evloop = tokio.spawn_event_loop('test')

    app = web.Application(debug=False, handler_args={'access_log': None})
    async def handler(req):
        return web.Response()

    app.router.add_get('/', handler)
    handler = app.make_handler(loop=evloop)

    server = evloop.create_server(ProtocolWrapper(handler), host="127.0.0.1", port=9090)
    # evloop.call_later(6000.0, stop_event_loop, name, evloop)

    print('starting', evloop.time())
    asyncio.get_event_loop().run_forever()

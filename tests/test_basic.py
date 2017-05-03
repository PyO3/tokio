# temp tests

import asyncio
import traceback
import tokio


def stop_event_loop(name, evloop, *args):
    print('stop_event_loop from: %s' % name, evloop, args)
    print(evloop.stop())


def cb2(name, handle, evloop):
    print('callback2: %s' % name, handle, 'running:', evloop.is_running())
    print(handle.cancel())
    print('callback2: %s' % name, evloop.stop(), evloop.time())


def test_call_later():
    name = 'test_call_later'
    evloop = tokio.new_event_loop()
    handle = evloop.call_later(1.0, stop_event_loop, name, evloop)
    evloop.call_later(0.5, cb2, name, handle, evloop)

    print(evloop, evloop.time())
    print('starting')
    evloop.run_forever()
    evloop.close()


def test_call_at():
    name = 'test_call_at'
    evloop = tokio.new_event_loop()
    time = evloop.time()

    handle = evloop.call_at(time + 1.0, stop_event_loop, name, evloop)
    evloop.call_at(time + 0.5, cb2, name, handle, evloop)

    print(evloop, evloop.time())
    evloop.run_forever()
    evloop.close()


def test_call_soon():
    name = 'test_call_soon'
    evloop = tokio.new_event_loop()

    evloop.call_soon(stop_event_loop, name, evloop)

    print('starting:', name, evloop, evloop.time())
    evloop.run_forever()
    evloop.close()


def create_fut(evloop):
    fut = evloop.create_future()
    fut.add_done_callback(cb_fut_res)
    evloop.call_later(0.5, cb_fut, 'call_later:fut', fut)


def cb_fut(name, fut):
    print(name, fut)
    fut.set_result(1)


def cb_fut_res(fut):
    print('future completed:', fut.result())


def test_future():
    evloop = tokio.new_event_loop()
    evloop.call_later(0.1, create_fut, evloop)
    evloop.call_later(2.0, stop_event_loop, 'test_future', evloop)

    print(evloop, evloop.time())
    print('starting')
    evloop.run_forever()
    evloop.close()
    print('done')


def test_future_exc():
    name = "test_future_exc"

    def create_fut(evloop):
        fut = evloop.create_future()

        try:
            fut.result()
        except Exception as exc:
            traceback.print_exc()
            print(repr(exc))

        fut.add_done_callback(cb_fut_res)
        fut.set_exception(ValueError())
        evloop.call_later(0.5, cb_fut, fut)

    def cb_fut(fut):
        print(name, fut)
        fut.set_result(1)

    def cb_fut_res(fut):
        print('future completed:', repr(fut.exception()))
        fut.result()

    evloop = tokio.new_event_loop()
    evloop.call_later(0.01, create_fut, evloop)
    evloop.call_later(2.0, stop_event_loop, name, evloop)

    print(evloop, evloop.time())
    print('starting')
    evloop.run_forever()
    evloop.close()


def test_task():
    name = 'test_task'
    counter = 0
    evloop = tokio.new_event_loop()

    async def coro():
        nonlocal counter
        while True:
            counter += 1
            print('coro', counter)
            await asyncio.sleep(0.2, loop=evloop)

    def start(evloop):
        asyncio.ensure_future(coro(), loop=evloop)

    evloop.call_later(0.01, start, evloop)
    evloop.call_later(3.0, stop_event_loop, name, evloop)

    print(evloop, evloop.time())
    print('starting')
    evloop.run_forever()
    evloop.close()

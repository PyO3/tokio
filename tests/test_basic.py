import tokio


def callback(name, evloop, *args):
    print ('callback: %s' % name, evloop, args, evloop.is_running())
    print(evloop.stop())


def cb2(name, handle, evloop):
    print ('callback2: %s' % name, handle, 'running:', evloop.is_running())
    print(handle.cancel())
    print('callback2: %s' % name, evloop.stop(), evloop.time())


def test_call_later():
    evloop = tokio.new_event_loop()
    handle = evloop.call_later(1.0, callback, 'call_later', evloop)
    evloop.call_later(0.5, cb2, 'call_later', handle, evloop)

    print(evloop, evloop.time())
    print('starting')
    evloop.run_forever()
    evloop.close()
    print('done')


def test_call_at():
    evloop = tokio.new_event_loop()
    time = evloop.time()

    handle = evloop.call_at(time + 1.0, callback, 'call_at', evloop)
    evloop.call_later(time + 0.5, cb2, 'call_at', handle, evloop)

    print(evloop, evloop.time())
    print('starting')
    evloop.run_forever()
    evloop.close()
    print('done')


def test_call_soon():
    evloop = tokio.new_event_loop()
    time = evloop.time()

    handle = evloop.call_soon(callback, 'call_soon', evloop)

    print(evloop, evloop.time())
    print('starting')
    evloop.run_forever()
    evloop.close()
    print('done')

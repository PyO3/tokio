import tokio


def callback(evloop, *args):
    print ('callback', evloop, args, evloop.is_running())
    print(evloop.stop())


def cb2(handle, evloop):
    print ('cb2', handle, 'running:', evloop.is_running())
    print(handle.cancel())
    print('cb2', evloop.stop(), evloop.time())


def test_basic():
    evloop = tokio.new_event_loop()
    handle = evloop.call_later(1.0, callback, evloop)
    evloop.call_later(0.5, cb2, handle, evloop)

    print(evloop, evloop.time())
    print('starting')
    evloop.run_forever()
    print('done')

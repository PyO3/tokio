
Asyncio event loop based on tokio-rs (WIP) |Build Status| |Join the dev chat at https://gitter.im/PyO3/Lobby|
=============================================================================================================

async-tokio is a drop-in replacement of the built-in asyncio event loop. async-tokio is implemented in rust and uses tokio-rs under the hood and `PyO3 python binding <https://github.com/PyO3/pyo3>`_.


Using tokio loop
----------------

You can create an instance of the loop manually, using:

.. code:: python

    import tokio
    
    policy = tokio.EventLoopPolicy()
    asyncio.set_event_loop_policy(policy)


Development of tokio loop
-------------------------

To build tokio loop, you'll need rust `nightly` and Python 3.6.  The best way
is to create a virtual env, so that you'll have ``python`` commands pointing to the correct tools.

1. ``git clone git@github.com:PyO3/tokio.git``

2. ``cd tokio``

3. ``make build``

4. ``make test``


Status
------

At the moment async-tokio works on unix like systems only.
Supported api:

* time api (call_at, call_later)
* sockets api (sock_xxx methods)
* tcp (client/server)
* unix domain socket
* dns
* pipes
* subprocess
* signals
* executors

UDP support is missing.


License
-------

``async-tokio`` is offered under the Apache 2.0 licenses.


.. |Build Status| image:: https://travis-ci.org/PyO3/tokio.svg?branch=master
                  :target: https://travis-ci.org/PyO3/tokio
.. |Join the dev chat at https://gitter.im/PyO3/Lobby| image:: https://img.shields.io/gitter/room/nwjs/nw.js.svg
   :target: https://gitter.im/PyO3/Lobby

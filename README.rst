Asyncio event loop based on tokio-rs (WIP)
==========================================

async-tokio is a drop-in replacement of the built-in asyncio event loop. async-tokio is implemented in rust and uses tokio-rs under the hood.


Using tokio loop
----------------

You can create an instance of the loop manually, using:

.. code:: python

    import tokio
    
    policy = tokio.TokioLoopPolicy()
    asyncio.set_event_loop_policy(policy)
    asyncio.set_event_loop(tokio.new_event_loop())


Development of tokio loop
-------------------------

To build tokio loop, you'll need rust 1.15.1+ and Python 3.6.  The best way
is to create a virtual env, so that you'll have ``python`` commands pointing to the correct tools.

1. ``git clone git@github.com:PyO3/tokio.git``

2. ``cd tokio``

3. ``make buid``

4. ``make test``


License
-------

``async-tokio`` is offered under the Apache 2.0 licenses.

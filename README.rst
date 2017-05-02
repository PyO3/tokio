Asyncio event loop based on tokio-rs (WIP)
==========================================

async-tokio is asyncio event loop written in rust.


Using tokio loop
----------------

You can create an instance of the loop manually, using:

.. code:: python

    import tokio
    
    loop = tokio.new_event_loop()
    asyncio.set_event_loop(loop)
    
    
Development of tokio loop
-------------------------

To build tokio loop, you'll need rust 1.15 and Python 3.5.  The best way
is to create a virtual env, so that you'll have ``cython`` and
``python`` commands pointing to the correct tools.

1. ``git clone git@github.com:PyO3/async-tokio.git``

2. ``cd async-tokio``

3. ``python ./setup.py develop``


License
-------

async-tokio is licensed Apache 2.0 licenses.

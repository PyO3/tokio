# Copied from the uvloop project.  If you add a new unittest here,
# please consider contributing it to the uvloop project.
#
# Portions copyright (c) 2015-present MagicStack Inc.  http://magic.io

import asyncio
import concurrent.futures

import pytest


def fib(n):
    if n < 2:
        return 1
    return fib(n - 2) + fib(n - 1)


@pytest.mark.parametrize(
    'pool_factory', [concurrent.futures.ProcessPoolExecutor,
                     concurrent.futures.ThreadPoolExecutor])
def test_executors_process_pool(loop, pool_factory):
    async def run():
        pool = pool_factory()
        with pool:
            coros = []
            for i in range(0, 10):
                coros.append(loop.run_in_executor(pool, fib, i))
                res = await asyncio.gather(*coros, loop=loop)
            assert res == fib10
            await asyncio.sleep(0.01, loop=loop)

    fib10 = [fib(i) for i in range(10)]
    loop.run_until_complete(run())

# Copied from the uvloop project.  If you add a new unittest here,
# please consider contributing it to the uvloop project.
#
# Portions copyright (c) 2015-present MagicStack Inc.  http://magic.io

import asyncio
import os
import signal
import subprocess
import sys
import time

import pytest

pytestmark = pytest.mark.skip("not fully implemented")


def test_process_env_1(loop2):
    async def test():
        cmd = 'echo $FOO$BAR'
        env = {'FOO': 'sp', 'BAR': 'am'}
        proc = await asyncio.create_subprocess_shell(
            cmd,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            loop=loop2)

        out, _ = await proc.communicate()
        assert out == b'spam\n'
        assert proc.returncode == 0

    loop2.run_until_complete(test())


def test_process_cwd_1(loop2):
    async def test():
        cmd = 'pwd'
        env = {}
        cwd = '/'
        proc = await asyncio.create_subprocess_shell(
            cmd,
            cwd=cwd,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            loop=loop2)

        out, _ = await proc.communicate()
        assert out == b'/\n'
        assert proc.returncode == 0

    loop2.run_until_complete(test())


def test_process_preexec_fn_1(loop2):
    # Copied from CPython/test_suprocess.py

    # DISCLAIMER: Setting environment variables is *not* a good use
    # of a preexec_fn.  This is merely a test.

    async def test():
        cmd = sys.executable
        proc = await asyncio.create_subprocess_exec(
            cmd, '-c',
            'import os,sys;sys.stdout.write(os.getenv("FRUIT"))',
            stdout=subprocess.PIPE,
            preexec_fn=lambda: os.putenv("FRUIT", "apple"),
            loop=loop2)

        out, _ = await proc.communicate()
        assert out == b'apple'
        assert proc.returncode == 0

    loop2.run_until_complete(test())


def test_process_preexec_fn_2(loop2):
    # Copied from CPython/test_suprocess.py

    def raise_it():
        raise ValueError("spam")

    async def test():
        cmd = sys.executable
        proc = await asyncio.create_subprocess_exec(
            cmd, '-c', 'import time; time.sleep(10)',
            preexec_fn=raise_it,
            loop=loop2)

        await proc.communicate()

    started = time.time()
    try:
        loop2.run_until_complete(test())
    except subprocess.SubprocessError as ex:
        assert 'preexec_fn' in ex.args[0]
        if ex.__cause__ is not None:
            # uvloop will set __cause__
            assert type(ex.__cause__) is ValueError
            assert ex.__cause__.args[0] == 'spam'
    else:
        assert False, 'exception in preexec_fn did not propagate to the parent'

    if time.time() - started > 5:
        assert False, 'exception in preexec_fn did not kill the child process'


def test_process_executable_1(loop2):
    async def test():
        proc = await asyncio.create_subprocess_exec(
            b'doesnotexist', b'-c', b'print("spam")',
            executable=sys.executable,
            stdout=subprocess.PIPE,
            loop=loop2)

        out, err = await proc.communicate()
        assert out == b'spam\n'

    loop2.run_until_complete(test())


def test_process_pid_1(loop2):
    async def test():
        prog = '''\
import os
print(os.getpid())
            '''

        cmd = sys.executable
        proc = await asyncio.create_subprocess_exec(
            cmd, b'-c', prog,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            loop=loop2)

        pid = proc.pid
        expected_result = '{}\n'.format(pid).encode()

        out, err = await proc.communicate()
        assert out == expected_result

    loop2.run_until_complete(test())


def test_process_send_signal_1(loop2):
    async def test():
        prog = '''\
import signal

def handler(signum, frame):
    if signum == signal.SIGUSR1:
        print('WORLD')

signal.signal(signal.SIGUSR1, handler)
a = input()
print(a)
a = input()
print(a)
exit(11)
        '''

        cmd = sys.executable
        proc = await asyncio.create_subprocess_exec(
            cmd, b'-c', prog,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            loop=loop2)

        proc.stdin.write(b'HELLO\n')
        await proc.stdin.drain()

        assert await proc.stdout.readline() == b'HELLO\n'

        proc.send_signal(signal.SIGUSR1)

        proc.stdin.write(b'!\n')
        await proc.stdin.drain()

        assert await proc.stdout.readline() == b'WORLD\n'
        assert await proc.stdout.readline() == b'!\n'
        assert await proc.wait() == 11

    loop2.run_until_complete(test())


def test_process_streams_basic_1(loop2):
    async def test():
        prog = '''\
import sys
while True:
    a = input()
    if a == 'stop':
        exit(20)
    elif a == 'stderr':
        print('OUCH', file=sys.stderr)
    else:
        print('>' + a + '<')
        '''

        cmd = sys.executable
        proc = await asyncio.create_subprocess_exec(
            cmd, b'-c', prog,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            loop=loop2)

        assert proc.pid > 0
        assert proc.returncode is None

        transp = proc._transport
        with pytest.raises(NotImplementedError):
            # stdin is WriteTransport
            transp.get_pipe_transport(0).pause_reading()
        with pytest.raises((NotImplementedError, AttributeError)):
            # stdout is ReadTransport
            transp.get_pipe_transport(1).write(b'wat')

        proc.stdin.write(b'foobar\n')
        await proc.stdin.drain()
        out = await proc.stdout.readline()
        assert out == b'>foobar<\n'

        proc.stdin.write(b'stderr\n')
        await proc.stdin.drain()
        out = await proc.stderr.readline()
        assert out == b'OUCH\n'

        proc.stdin.write(b'stop\n')
        await proc.stdin.drain()

        exitcode = await proc.wait()
        assert exitcode == 20

    loop2.run_until_complete(test())


def test_process_streams_stderr_to_stdout(loop2):
    async def test():
        prog = '''\
import sys
print('out', flush=True)
print('err', file=sys.stderr, flush=True)
        '''

        proc = await asyncio.create_subprocess_exec(
            sys.executable, '-c', prog,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            loop=loop2)

        out, err = await proc.communicate()
        assert err is None
        assert out == b'out\nerr\n'

    loop2.run_until_complete(test())

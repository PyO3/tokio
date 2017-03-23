import sys
import os.path
import subprocess
from setuptools import setup
from setuptools_rust import RustExtension, build_ext
from setuptools.command.test import test as TestCommand


class PyTest(TestCommand):
    user_options = []

    def run(self):
        import subprocess
        import sys
        errno = subprocess.call([sys.executable, '-m', 'pytest', 'tests'])
        raise SystemExit(errno)


install_requires = []
tests_require = install_requires + ['pytest', 'pytest-timeout']


setup(**dict(
    name='async-tokio',
    version='0.0.1',
    author='Nikolay Kim',
    author_email='fafhrd91@gmail.com',
    url='https://github.com/aio-libs/async-tokio/',
    packages=['tokio'],
    rust_extensions=[
        RustExtension('tokio._ext', 'ext/Cargo.toml')],
    install_requires=install_requires,
    tests_require=tests_require,
    include_package_data=True,
    zip_safe=False,
    cmdclass=dict(test=PyTest),
))

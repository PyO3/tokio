import codecs
import re
import os
import sys
from setuptools import setup
from setuptools.command.test import test as TestCommand

try:
    from setuptools_rust import Binding, RustExtension
except ImportError:
    import subprocess
    import sys
    errno = subprocess.call([sys.executable, '-m', 'pip', 'install', 'setuptools-rust'])
    if errno:
        print("Please install setuptools-rust package")
        raise SystemExit(errno)
    else:
        from setuptools_rust import Binding, RustExtension


class PyTest(TestCommand):
    user_options = []

    def run(self):
        self.run_command("test_rust")

        import subprocess
        import sys
        errno = subprocess.call([sys.executable, '-m', 'pytest', 'tests'])
        raise SystemExit(errno)


with codecs.open(os.path.join(os.path.abspath(os.path.dirname(
        __file__)), 'tokio', '__init__.py'), 'r', 'latin1') as fp:
    try:
        version = re.findall(r"^__version__ = '([^']+)'\r?$",
                             fp.read(), re.M)[0]
    except IndexError:
        raise RuntimeError('Unable to determine version.')


setup_requires = ['setuptools-rust>=0.6.1']
install_requires = []
tests_require = install_requires + ['pytest', 'pytest-timeout']


if sys.version_info < (3, 6, 0):
    raise RuntimeError("tokio requires Python 3.6+")


def read(f):
    return open(os.path.join(os.path.dirname(__file__), f)).read().strip()


setup(name='tokio',
      version=version,
      description='Asyncio event loop written in Rust language',
      long_description='\n\n'.join((read('README.rst'), read('CHANGES.rst'))),
      license='Apache 2',
      classifiers=[
          'License :: OSI Approved :: Apache Software License',
          'Development Status :: 3 - Alpha',
          'Framework :: AsyncIO',
          'Intended Audience :: Developers',
          'Programming Language :: Python',
          'Programming Language :: Python :: 3 :: Only',
          'Programming Language :: Python :: 3.6',
          'Programming Language :: Rust',
          'Operating System :: POSIX',
          'Operating System :: MacOS :: MacOS X',
          'Topic :: Internet :: WWW/HTTP',
      ],
      author='Nikolay Kim',
      author_email='fafhrd91@gmail.com',
      url='https://github.com/PyO3/tokio/',
      packages=['tokio'],
      rust_extensions=[RustExtension('tokio._tokio', 'Cargo.toml', binding=Binding.PyO3)],
      install_requires=install_requires,
      tests_require=tests_require,
      setup_requires=setup_requires,
      include_package_data=True,
      zip_safe=False,
      cmdclass=dict(test=PyTest))

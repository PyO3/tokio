import codecs
import re
import os
import sys
from setuptools import setup
from setuptools.command.test import test as TestCommand

try:
    from setuptools_rust import RustExtension
except ImportError:
    import subprocess
    import sys
    errno = subprocess.call([sys.executable, '-m', 'pip', 'install', 'setuptools-rust'])
    if errno:
        print("Please install setuptools-rust package")
        raise SystemExit(errno)
    else:
        from setuptools_rust import RustExtension


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


setup_requires = ['setuptools-rust>=0.5.1']
install_requires = []
tests_require = install_requires + ['pytest', 'pytest-timeout']


if sys.version_info < (3, 6, 0):
    raise RuntimeError("tokio requires Python 3.6+")


setup(name='tokio',
      version=version,
      classifiers=[
          'License :: OSI Approved :: Apache Software License',
          'Intended Audience :: Developers',
          'Programming Language :: Python',
          'Programming Language :: Python :: 3 :: Only',
          'Programming Language :: Python :: 3.6',
          'Operating System :: POSIX',
          'Operating System :: MacOS :: MacOS X',
          'Topic :: Internet :: WWW/HTTP',
          'Framework :: AsyncIO',
      ],
      author='Nikolay Kim',
      author_email='fafhrd91@gmail.com',
      url='https://github.com/PyO3/tokio/',
      packages=['tokio'],
      rust_extensions=[RustExtension('tokio._tokio', 'Cargo.toml')],
      install_requires=install_requires,
      tests_require=tests_require,
      setup_requires=setup_requires,
      include_package_data=True,
      zip_safe=False,
      cmdclass=dict(test=PyTest))

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


setup_requires = ['setuptools-rust>=0.4.2']
install_requires = []
tests_require = install_requires + ['pytest', 'pytest-timeout']


setup(name='tokio',
      version='0.0.1',
      author='Nikolay Kim',
      author_email='fafhrd91@gmail.com',
      url='https://github.com/PyO3/async-tokio/',
      # packages=['tokio'],
      rust_extensions=[RustExtension('tokio', 'Cargo.toml')],
      install_requires=install_requires,
      tests_require=tests_require,
      setup_requires=setup_requires,
      include_package_data=True,
      zip_safe=False,
      cmdclass=dict(test=PyTest))

# import tokio
from pprint import pprint


def test_basic():
    from tokio import _ext
    print(_ext)
    pprint(dir(_ext))

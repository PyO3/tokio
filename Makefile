# Some simple testing tasks (sorry, UNIX only).

.install-deps: requirements-dev.txt
	@pip install -U -r requirements-dev.txt
	@touch .install-deps

isort:
	isort -rc tokio
	isort -rc tests

flake: .flake

.flake: .install-deps $(shell find tokio -type f) \
                      $(shell find tests -type f)
	@flake8 tokio tests
	python setup.py check -rms;
	@if ! isort -c -rc tokio tests; then \
            echo "Import sort errors, run 'make isort' to fix them!!!"; \
            isort --diff -rc tokio tests; \
            false; \
	fi
	@touch .flake


.develop: .install-deps $(shell find tokio -type f) .flake
	@pip install -e .
	@touch .develop

test: .develop
	@py.test -q ./tests

vtest: .develop
	@py.test -s -v ./tests

cov cover coverage:
	tox

cov-dev: .develop
	@py.test --cov=tokio --cov-report=term --cov-report=html tests
	@echo "open file://`pwd`/coverage/index.html"

build:
	@python setup.py build_rust --inplace --debug

clean:
	@rm -rf `find . -name __pycache__`
	@rm -f `find . -type f -name '*.py[co]' `
	@rm -f `find . -type f -name '*~' `
	@rm -f `find . -type f -name '.*~' `
	@rm -f `find . -type f -name '@*' `
	@rm -f `find . -type f -name '#*#' `
	@rm -f `find . -type f -name '*.orig' `
	@rm -f `find . -type f -name '*.rej' `
	@rm -f .coverage
	@rm -rf coverage
	@rm -rf build
	@rm -rf cover
	@make -C docs clean
	@python setup.py clean
	@rm -rf .tox

doc:
	@make -C docs html SPHINXOPTS="-W -E"
	@echo "open file://`pwd`/docs/_build/html/index.html"

doc-spelling:
	@make -C docs spelling SPHINXOPTS="-W -E"

install:
	@pip install -U pip
	@pip install -Ur requirements-dev.txt

.PHONY: all build flake test vtest cov clean doc

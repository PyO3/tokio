# Some simple testing tasks (sorry, UNIX only).

.install-deps: requirements-dev.txt requirements-ci.txt
	@pip install -U -r requirements-dev.txt
	@touch .install-deps

isort:
# isort -rc tokio
	isort -rc tests

flake: .flake

.flake: .install-deps .build $(shell find tokio -type f) $(shell find tests -type f)
	@flake8 tokio tests
	python setup.py check -rms
	@if ! isort -c -rc tokio tests; then \
            echo "Import sort errors, run 'make isort' to fix them!!!"; \
            isort --diff -rc tokio tests; \
            false; \
	fi
	@touch .flake

.develop: .install-deps .build $(shell find tokio -type f)
	@pip install -e .
	@touch .develop

test: .develop .flake .build
	@cargo test --color always
	@python -m pytest ./tests

build:
	@python setup.py develop

.build: setup.py $(shell find src -type f)
	@python setup.py develop
	@touch .build

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

install:
	@pip install -U pip
	@pip install -Ur requirements-dev.txt

.PHONY: all build flake test vtest cov clean

FROM docker.io/library/rust:1-bookworm

RUN apt-get update \
&&  apt-get install -y --no-install-recommends libasound2-dev pkg-config \
&&  apt-get clean \
&&  rm -rf /var/lib/apt/lists/*

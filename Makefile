DOCKER_NAME ?= rcore-tutorial-v3
.PHONY: docker build_docker report
ID = $(shell git rev-parse --abbrev-ref HEAD | grep -oP 'ch\K[0-9]')
	
docker:
	docker run --rm -it -v ${PWD}:/mnt -w /mnt ${DOCKER_NAME} bash

build_docker: 
	docker build -t ${DOCKER_NAME} .

fmt:
	cd os ; cargo fmt;  cd ..

report:
	mkdir -p reports
	echo "# report" > "reports/lab$(ID).md"
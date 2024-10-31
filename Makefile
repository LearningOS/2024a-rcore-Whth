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
	for i in $(shell seq 1 $(ID)); do \
		F_NAME=reports/lab$$i.md; \
		echo "## lab$$i" >> "$$F_NAME"; \
	done

clean:
	rm -rf user ci-user

checker: clean
	git clone https://ghp.ci/https://github.com/LearningOS/rCore-Tutorial-Test-2024A user --depth 1
	git clone https://ghp.ci/https://github.com/LearningOS/rCore-Tutorial-Checker-2024A.git ci-user --depth 1
	cp -r user ci-user/user
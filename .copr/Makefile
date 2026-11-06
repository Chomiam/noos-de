# COPR, méthode « make srpm » : https://docs.pagure.org/copr.copr/user_documentation.html#make-srpm
# COPR appelle : make -f .copr/Makefile srpm outdir=… spec=…
outdir ?= $(CURDIR)/packaging/rpm/out

srpm:
	dnf -y install git-core cargo rpm-build tar xz
	git config --global --add safe.directory '*'
	./packaging/rpm/make-srpm.sh $(outdir)

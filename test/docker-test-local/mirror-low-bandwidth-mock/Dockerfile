FROM nginx:1.17.10

COPY pkg.tar /tmp
RUN tar -C /usr/share/nginx/html -xf /tmp/pkg.tar

COPY default.conf /etc/nginx/conf.d/default.conf

RUN dd if=/dev/zero of=/usr/share/nginx/html/zero bs=30M count=1

COPY create_test_files /root
RUN /root/create_test_files

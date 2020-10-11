FROM nginx:1.19.1-alpine

COPY pkg.tar /tmp
RUN tar -C /usr/share/nginx/html -xf /tmp/pkg.tar

RUN truncate -s 8589934592 /usr/share/nginx/html/zero

COPY create_test_files /root
RUN /root/create_test_files

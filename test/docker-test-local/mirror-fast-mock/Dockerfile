FROM nginx:1.19.1

COPY pkg.tar /tmp
RUN tar -C /usr/share/nginx/html -xf /tmp/pkg.tar

RUN truncate -s 31457280 /usr/share/nginx/html/zero

RUN truncate -s 8589934592 /usr/share/nginx/html/large

RUN truncate -s 100 /usr/share/nginx/html/partially-cached

COPY create_test_files /root
RUN /root/create_test_files

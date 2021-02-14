FROM nginx:1.17.10

COPY pkg.tar /tmp
RUN tar -C /usr/share/nginx/html -xf /tmp/pkg.tar

RUN truncate -s 31457280 /usr/share/nginx/html/zero

COPY default.conf /etc/nginx/conf.d/default.conf

COPY create_test_files /root
RUN /root/create_test_files

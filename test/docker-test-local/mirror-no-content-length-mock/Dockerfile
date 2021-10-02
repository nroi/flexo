FROM openresty/openresty:1.19.9.1-1-buster-fat

RUN mkdir -p /usr/share/nginx/html

COPY default.conf /etc/nginx/conf.d/default.conf

COPY create_test_files /root

RUN /root/create_test_files

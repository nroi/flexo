FROM openresty/openresty:buster-fat

COPY pkg.tar /tmp
RUN tar -C /usr/local/openresty/nginx/html -xf /tmp/pkg.tar

COPY default.conf /etc/nginx/conf.d/default.conf

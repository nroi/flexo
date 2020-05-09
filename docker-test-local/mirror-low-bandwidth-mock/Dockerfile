FROM nginx:1.17.10

COPY pkg.tar /tmp
RUN tar -C /usr/share/nginx/html -xf /tmp/pkg.tar

COPY default.conf /etc/nginx/conf.d/default.conf

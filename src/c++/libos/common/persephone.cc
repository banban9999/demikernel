#include <iostream>

#include <yaml-cpp/yaml.h>
#include <dmtr/libos.h>
#include <dmtr/libos/persephone.hh>

#include <dmtr/annot.h>

/********** CONTROL PLANE ******************/
Psp::Psp(std::string &app_cfg) {
    /* Let network libOS init its specific EAL */
    dmtr_net_init(app_cfg.c_str());

    /* Allocate a mempool for the network */
    net_ctx.net_mempool = NULL;
    dmtr_net_mempool_init(&net_ctx.net_mempool, 0);

    /* Setup the fragmentation context */
    //TODO

    /* Parse the configuration */
    std::unordered_map<uint16_t, uint32_t> devices_to_sus;
    try {
        YAML::Node config = YAML::LoadFile(app_cfg);
        YAML::Node sus = config["service_units"];
        for (size_t i = 0; i < sus.size(); ++i) {
            std::shared_ptr<PspServiceUnit> service_unit = std::make_shared<PspServiceUnit>(i);
            for (auto su = sus[i].begin(); su != sus[i].end(); ++su) {
                auto key = su->first.as<std::string>();
                if (key == "io") {
                    for (size_t j = 0; j < su->second.size(); ++j) {
                        for (auto ioq = su->second[j].begin(); ioq != su->second[j].end(); ++ioq) {
                            auto ioq_key = ioq->first.as<std::string>();
                            if (ioq_key == "type" && ioq->second.as<std::string>() == "NETWORK_Q") {
                                auto dev_id = su->second[j]["device_id"].as<uint16_t>();
                                auto it = devices_to_sus.find(dev_id);
                                if (it == devices_to_sus.end()) {
                                    devices_to_sus.insert(
                                        std::pair<uint16_t, uint32_t>(dev_id, 1)
                                    );
                                } else {
                                    devices_to_sus[dev_id]++;
                                }
                                dmtr_init_net_context(&service_unit->io_ctx->net_context);
                            }
                        }
                    }
                }
            }
            service_units.push_back(service_unit);
        }
        YAML::Node cfg_log_dir = config["log_dir"];
        log_dir = cfg_log_dir.as<std::string>();
    } catch (YAML::ParserException& e) {
        std::cout << "Failed to parse config: " << e.what() << std::endl;
        exit(1);
    }

    /** Configure the network interface itself
     * (with as many rx/tx queue than we have service units using the device)
     */
    for (auto &d: devices_to_sus) {
        dmtr_net_port_init(d.first, net_ctx.net_mempool, d.second, d.second);
    }
}

/************** SERVICE UNITS ************/
int PspServiceUnit::socket(int &qd, int domain, int type, int protocol) {

    DMTR_OK(ioqapi.socket(qd, domain, type, protocol));
    DMTR_OK(ioqapi.set_io_ctx(qd, io_ctx->net_context));

    return 0;
}

#define WAIT_MAX_ITER 10000

int PspServiceUnit::wait(dmtr_qresult_t *qr_out, dmtr_qtoken_t qt) {
    int ret = EAGAIN;
    uint16_t iter = 0;
    while (EAGAIN == ret) {
        if (iter++ == WAIT_MAX_ITER) {
            return EAGAIN;
        }
        ret = ioqapi.poll(qr_out, qt);
    }
    DMTR_OK(ioqapi.drop(qt));
    return ret;
}

int PspServiceUnit::wait_any(dmtr_qresult_t *qr_out, int *start_offset, int *ready_offset, dmtr_qtoken_t qts[], int num_qts) {    uint16_t iter = 0;
    while (1) {
        for (int i = start_offset? *start_offset : 0; i < num_qts; i++) {
            int ret = ioqapi.poll(qr_out, qts[i]);
            if (ret != EAGAIN) {
                if (ret == 0 || ret == ECONNABORTED || ret == ECONNRESET) {
                    DMTR_OK(ioqapi.drop(qts[i]));
                    if (ready_offset != NULL) {
                        *ready_offset = i;
                    }
                    if (start_offset != NULL && *start_offset != 0) {
                        *start_offset = 0;
                    }
                    return ret;
                }
            } else {
                if (iter++ == WAIT_MAX_ITER) {
                    if (start_offset != NULL) {
                        *start_offset = i;
                    }
                    return EAGAIN;
                }
            }
        }
        *start_offset = 0;
    }
    DMTR_UNREACHABLE();
}